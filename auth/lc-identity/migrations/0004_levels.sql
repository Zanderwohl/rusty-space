-- Permission becomes a level rather than a flag.
--
-- **Lower is higher.** 0 is a player; 1, 2 and 3 are administrators with 1 the most senior.
-- The inversion is deliberate and it is the one thing about this column that is easy to get
-- wrong: `permission >= 1` reads as "is an administrator" and is right, while `permission >
-- other` reads as "outranks" and is exactly backwards. Nothing above this file compares the
-- integers directly -- `lc_identity::level::Level` is the type that carries the ordering, and
-- `lc_server::ability::Level` is its opposite number across the workspace boundary.
--
-- Nothing migrates. The one account at 1 was the only administrator under the old flag and is
-- the most senior one under the new levels, which is the same person with the same powers.

alter table accounts add constraint accounts_permission_is_a_level
    check (permission between 0 and 3);

comment on column accounts.permission is
    '0 player, 1..3 administrator with 1 the most senior. Lower outranks higher.';
