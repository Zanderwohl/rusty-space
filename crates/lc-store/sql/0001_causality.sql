-- Causality, in the database.
--
-- The same predicate `lc-spacetime` computes in Rust, so that a query and a client agree about
-- what may have influenced what. Two implementations of causality is one too many; the pair is
-- held together by a randomised agreement test rather than by inspection.
--
-- See lightcone/docs/02-event-store.md and lightcone/docs/01-spacetime.md.

-- The invariant interval squared, `dt^2 - dr^2`, in squared microseconds.
--
-- `numeric` because the squares overflow bigint and Postgres has no 128-bit integer. The 2^60
-- coordinate bound is what keeps the result exact: it bounds every difference by 2^61, so the
-- squares need 122 bits and `numeric` carries them without rounding.
--
-- Slow -- `numeric` multiplication is of order a hundred times a float8 one -- so this is for
-- the exact test after an index prune, never inside a scan.
--
-- Multiplied rather than raised to a power. `numeric ^ 2` gives the same value, but through
-- `numeric_power`, which picks a display scale: a difference of one comes back as
-- `-1.0000000000000000`. Multiplication of two scale-zero numerics is scale zero, so the result
-- reads as the integer it is, and it is the cheaper of the two besides.
CREATE OR REPLACE FUNCTION lc_interval2(
    t1 bigint, x1 bigint, y1 bigint, z1 bigint,
    t2 bigint, x2 bigint, y2 bigint, z2 bigint
) RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT AS $$
    SELECT (t2::numeric - t1::numeric) * (t2::numeric - t1::numeric)
         - ( (x2::numeric - x1::numeric) * (x2::numeric - x1::numeric)
           + (y2::numeric - y1::numeric) * (y2::numeric - y1::numeric)
           + (z2::numeric - z1::numeric) * (z2::numeric - z1::numeric) );
$$;

DO $$ BEGIN
    CREATE TYPE lc_separation AS ENUM ('timelike', 'lightlike', 'spacelike');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- How two events are separated. Frame-independent because nothing here moves faster than
-- light -- not unconditionally.
CREATE OR REPLACE FUNCTION lc_classify(
    t1 bigint, x1 bigint, y1 bigint, z1 bigint,
    t2 bigint, x2 bigint, y2 bigint, z2 bigint
) RETURNS lc_separation
LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT AS $$
    SELECT CASE sign(lc_interval2(t1, x1, y1, z1, t2, x2, y2, z2))
        WHEN  1 THEN 'timelike'
        WHEN  0 THEN 'lightlike'
        ELSE         'spacelike'
    END::lc_separation;
$$;

-- True when the first event could have influenced the second.
--
-- The only ordering a rule may depend on: reflexive, antisymmetric, transitive and
-- frame-independent, so it means the same thing to every observer.
CREATE OR REPLACE FUNCTION lc_precedes(
    t1 bigint, x1 bigint, y1 bigint, z1 bigint,
    t2 bigint, x2 bigint, y2 bigint, z2 bigint
) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT AS $$
    SELECT t2 >= t1 AND lc_interval2(t1, x1, y1, z1, t2, x2, y2, z2) >= 0;
$$;

-- There is deliberately no `lc_simultaneous`. Equal `t` is `WHERE t1 = t2`; dressing a frame
-- convention up as a physics predicate would invite rules that depend on it, and the order of
-- a spacelike pair can be reversed by a boost.
