/**
 * The address bar.
 *
 * The console's state lives in the URL. The server renders from it and builds every link with
 * it; what it cannot do is *write* it, because an htmx swap does not navigate. That is this
 * module's whole job and the reason any TypeScript is here at all.
 *
 * **Canonical is the server's to define** — the defaults arrive in `data-listing`, written by
 * the same table `query_string` drops parameters against, so there is one copy of them.
 *
 * **Nothing here is required.** Scripting off, every control is a real link or GET form.
 */

import type { Htmx } from "htmx.org";

declare global {
  interface Window {
    htmx?: Htmx;
  }
}

/** What the server tells the browser about this listing. */
interface Listing {
  page: string;
  partial: string;
  /** Id of the element the partial replaces. */
  target: string;
  /** In the order the canonical string writes them. */
  order: string[];
  defaults: Record<string, string>;
}

/** Without this, restoring pushes a new entry instead of consuming one and Back never exits. */
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

/** The rule `Listing::query_string` applies on the server, so the two produce the same bytes. */
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
 * The pager and the headings change `sort`, `dir` and `page` without touching the form, so its
 * hidden fields go stale and the next filter change would undo the sort.
 */
function hydrate(form: HTMLFormElement, params: URLSearchParams, listing: Listing): void {
  for (const key of listing.order) {
    const wanted = params.get(key) ?? listing.defaults[key] ?? "";
    for (const field of form.elements) {
      if (!(field instanceof HTMLInputElement || field instanceof HTMLSelectElement)) continue;
      if (field.name !== key) continue;
      // Assigning an unchanged value moves the caret to the end in some browsers.
      if (field.value !== wanted) field.value = wanted;
    }
  }
}


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

  // Insurance against a browser restoring its own idea of what the controls held.
  hydrate(form, new URLSearchParams(location.search), listing);

  document.body.addEventListener("htmx:after:swap", (event) => {
    // **`event.target` is the element that issued the request**, not the one replaced —
    // that is `detail.ctx.target`. Keying on the former type-checks and silently does nothing
    // for every interaction that came from the form.
    const swapped = event.detail?.ctx?.target;
    if (!(swapped instanceof Element) || swapped.id !== listing.target) return;

    // Off the document, not the node the event handed over: with `outerHTML` they are not
    // reliably the same, and the event's may carry the previous URL.
    const live = document.getElementById(listing.target);
    const canonicalUrl = live?.getAttribute("data-canonical");
    if (!canonicalUrl) return;

    remember(canonicalUrl, !restoring);
    restoring = false;
    hydrate(form, new URL(canonicalUrl, location.origin).searchParams, listing);
  });

  // Written before the request comes back, so a URL copied mid-flight names what was asked
  // for rather than what is still on screen.
  form.addEventListener("change", () => {
    const params = harvest(form, listing);
    // A new filter starts at the first page, as the absent parameter does on the server.
    params.delete("page");
    remember(withQuery(listing.page, canonical(params, listing)), false);
  });

  addEventListener("popstate", () => {
    const params = new URLSearchParams(location.search);
    hydrate(form, params, listing);
    const htmx = window.htmx;
    if (!htmx) {
      // No htmx, no swap.
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

  // Back/forward cache: never re-rendered, so the controls hold whatever the browser put back.
  addEventListener("pageshow", (event) => {
    if (event.persisted) hydrate(form, new URLSearchParams(location.search), listing);
  });
}
