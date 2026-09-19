/**
 * The address bar.
 *
 * The console's state — every filter, the sort, the page — lives in the URL and nowhere else.
 * The server renders from it and builds every link with it; what it cannot do is *write* it,
 * because after an htmx swap the document did not navigate. That is this module's whole job,
 * and it is the reason there is any TypeScript here at all.
 *
 * Three things follow from taking that job:
 *
 * - **What "canonical" means is the server's to decide.** The defaults come down in
 *   `data-listing` on the filter form, written by `Listing::defaults_json` — the same table
 *   `Listing::query_string` drops parameters against. Two copies of "the default sort is name"
 *   is two places to change it and one place to forget.
 * - **Back and forward have to work.** A filter change that pushes history and then cannot
 *   restore it is worse than one that never touched history at all.
 * - **Nothing here is required.** With scripting off, every control is a real link or a real
 *   GET form pointed at the same URLs, and the console works a page load at a time.
 */

import type { Htmx } from "htmx.org";

declare global {
  interface Window {
    htmx?: Htmx;
  }
}

/** What the server tells the browser about this listing. */
interface Listing {
  /** Where the address bar should read. */
  page: string;
  /** Where the swappable part is fetched from. */
  partial: string;
  /** The id of the element the partial replaces. */
  target: string;
  /** Parameter names, in the order the canonical string writes them. */
  order: string[];
  /** The value each parameter has when it is absent. */
  defaults: Record<string, string>;
}

/**
 * True while a history entry is being restored.
 *
 * The restore issues a request, the request lands a swap, and the swap handler pushes
 * history — so without this, going back adds an entry instead of consuming one and the back
 * button never reaches the page before the console.
 */
let restoring = false;

function listingForm(): HTMLFormElement | null {
  return document.querySelector<HTMLFormElement>("form[data-listing]");
}

function listingOf(form: HTMLFormElement): Listing | null {
  const raw = form.dataset["listing"];
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Listing;
  } catch {
    // A malformed blob is a bug in the server, not something to take the console down over:
    // without this module every control still works as a link.
    console.warn("lc-admin: the listing configuration did not parse");
    return null;
  }
}

/**
 * The canonical query string for `params`: every value that differs from its default, in the
 * configured order, and nothing else.
 *
 * The same rule `Listing::query_string` applies on the server, which is what lets the two
 * produce the same bytes — and why the defaults are read from the page rather than written
 * out again here.
 */
export function canonical(params: URLSearchParams, listing: Listing): string {
  const out = new URLSearchParams();
  for (const key of listing.order) {
    const value = params.get(key);
    if (value === null) continue;
    if (value === listing.defaults[key]) continue;
    out.set(key, value);
  }
  return out.toString();
}

function withQuery(path: string, query: string): string {
  return query ? `${path}?${query}` : path;
}

/** The controls' current values, as parameters. */
function harvest(form: HTMLFormElement, listing: Listing): URLSearchParams {
  const params = new URLSearchParams();
  const data = new FormData(form);
  for (const key of listing.order) {
    const value = data.get(key);
    if (typeof value === "string") params.set(key, value);
  }
  return params;
}

/**
 * Put the URL's values into the controls.
 *
 * Needed after a swap — the pager and the column headings change `sort`, `dir` and `page`
 * without touching the form, so the hidden fields that carry them would otherwise go stale and
 * the next filter change would undo the sort. Needed again on a back/forward restore, and on a
 * page restored from the back/forward cache, where the browser may have kept the values the
 * controls had rather than the ones the URL asks for.
 */
function hydrate(form: HTMLFormElement, params: URLSearchParams, listing: Listing): void {
  for (const key of listing.order) {
    const wanted = params.get(key) ?? listing.defaults[key] ?? "";
    for (const field of form.elements) {
      if (!(field instanceof HTMLInputElement || field instanceof HTMLSelectElement)) continue;
      if (field.name !== key) continue;
      // Assigning an unchanged value still fires nothing, but it does move a text caret to
      // the end in some browsers. The guard is what keeps typing in the search box smooth
      // while its own results land.
      if (field.value !== wanted) field.value = wanted;
    }
  }
}

/** Write `url` to the address bar without navigating. */
function remember(url: string, push: boolean): void {
  const at = location.pathname + location.search;
  if (url === at) return;
  if (push) history.pushState(null, "", url);
  else history.replaceState(null, "", url);
}

export function install(): void {
  const form = listingForm();
  if (!form) return;
  const listing = listingOf(form);
  if (!listing) return;

  // A deep link, or a reload. The server has already rendered the controls from these, so
  // this is only insurance against a browser restoring its own idea of what they held.
  hydrate(form, new URLSearchParams(location.search), listing);

  document.body.addEventListener("htmx:after:swap", (event) => {
    const swapped = event.target;
    if (!(swapped instanceof Element) || swapped.id !== listing.target) return;

    // The server says where the address bar should point: it owns what canonical means, and
    // the swap it just answered may have come from a link rather than from these controls.
    const canonicalUrl = swapped.getAttribute("data-canonical");
    if (!canonicalUrl) return;

    remember(canonicalUrl, !restoring);
    restoring = false;
    hydrate(form, new URL(canonicalUrl, location.origin).searchParams, listing);
  });

  // The controls write the address bar as soon as they are touched, before the request that
  // they triggered has come back. A URL copied mid-request is then the URL of what was asked
  // for rather than of what is still on screen.
  form.addEventListener("change", () => {
    const params = harvest(form, listing);
    // Any filter change starts again at the first page, which is also what the server does
    // with the absent parameter. Saying it here keeps the address bar from claiming page 7 of
    // a list that has just been narrowed to two rows.
    params.delete("page");
    remember(withQuery(listing.page, canonical(params, listing)), false);
  });

  addEventListener("popstate", () => {
    const params = new URLSearchParams(location.search);
    hydrate(form, params, listing);
    const htmx = window.htmx;
    if (!htmx) {
      // No htmx, no swap: the honest answer is the page the URL names.
      location.reload();
      return;
    }
    restoring = true;
    void htmx
      .ajax("GET", withQuery(listing.partial, canonical(params, listing)), {
        target: `#${listing.target}`,
        swap: "outerHTML",
      })
      .catch(() => {
        restoring = false;
      });
  });

  // Restored from the back/forward cache. The document was never re-rendered, so the controls
  // hold whatever the browser decided to put back in them.
  addEventListener("pageshow", (event) => {
    if (event.persisted) hydrate(form, new URLSearchParams(location.search), listing);
  });
}
