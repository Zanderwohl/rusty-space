-- Where each account is in each book it has opened.
--
-- A checkpoint, like `ships` and unlike everything else in this schema: the rest holds events a
-- worldline can be reconstructed from, and this holds an answer as of a moment. Nothing in the
-- world depends on it, so it is never replayed and never pruned by `retention`.
--
-- The locator is a character offset and **not a page**. A page is a fact about a window at a
-- size; this survives a font change, a resize, and reading the same book on another machine.
-- See lightcone/docs/21-library.md.

CREATE TABLE IF NOT EXISTS reading (
    account     text NOT NULL,
    -- The catalogue's id, not a file name. The file a book is served from can change without
    -- the book changing; two of the first six were named after a Gutenberg number.
    book        text NOT NULL,
    -- Which spine document, and how far into its text.
    spine       integer NOT NULL,
    char_offset integer NOT NULL,
    -- How far through, for a shelf that wants to say "34%" without fetching the book.
    --
    -- **Reported by the client and not checked.** The server does not have the epub and cannot
    -- derive this; it also does not need to, because nothing is gated on reading and there is
    -- nothing to win by lying. A player who lies here fools only their own shelf.
    location    integer NOT NULL,
    locations   integer NOT NULL,
    -- What "recently read" is ordered by. The server's clock, because it is the only one both
    -- ends can agree on and nothing here needs better than that.
    read_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account, book)
);

CREATE INDEX IF NOT EXISTS reading_recent ON reading (account, read_at DESC);
