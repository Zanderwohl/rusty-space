-- An upstream dance in progress.
--
-- The broker redirects a browser to Google and gets it back some seconds later, by which time
-- the only thing tying the two halves together is the `state` parameter. What has to survive
-- the gap is what the callback cannot re-derive: the PKCE verifier, and where the sign-in was
-- headed before it was interrupted.
--
-- A table rather than a cookie or process memory. A cookie would be a second thing to get
-- SameSite right about on a cross-site redirect, and process memory would mean a dance that
-- only completes if it lands on the replica that started it.
create table upstream_flows (
    -- The digest of the state parameter, never the state. Consistent with signin_codes and for
    -- the same reason: knowing a pending state is enough to complete somebody else's dance.
    digest           bytea       primary key,
    provider         text        not null,
    -- The PKCE verifier, in the clear because the token request needs it back. It is worth
    -- nothing without the authorization code, which is delivered once, elsewhere, and to us.
    verifier         text        not null,
    -- Where the whole sign-in was going, checked against the allowlist when the dance started
    -- and **again** when it finishes.
    return_to        text        not null,
    -- The caller's own nonce, held for the length of the dance and handed back untouched.
    downstream_state text        not null,
    expires_at       timestamptz not null
);

create index upstream_flows_expiry on upstream_flows (expires_at);
