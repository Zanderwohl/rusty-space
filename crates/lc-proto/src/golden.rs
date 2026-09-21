//! The bytes each message of this protocol version encodes to.
//!
//! A format that is not self-describing cannot notice a field that moved, so this is what
//! notices: change the shape of anything in [`super`] without bumping
//! [`super::PROTOCOL_VERSION`] and the test on these fails. A deployed client would otherwise
//! read the new shape as the old one and be confidently wrong rather than refused.
//!
//! These are **data**, and they live apart from the types for that reason alone.

/// `Outbound::Welcome { .., ship: Motion { at [4.2, 0, 0], holding a 12 Mm orbit of Earth } }`
pub const WELCOME: &[u8] = &[
    0, 7, 29, 84, 128, 137, 122, 3, 65, 100, 97, 0, 0, 0, 0, 0, 0, 240, 63, 205, 204,
    204, 204, 204, 204, 16, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24,
    245, 64, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 3, 1, 1, 5, 69, 97, 114,
    116, 104, 0, 0, 0, 0, 96, 227, 102, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 224, 63,
];

/// `Inbound::Act(Intent { ship_id: 42, order: Transmit { power_w: 1500.0 }, .. })`
pub const ACT: &[u8] = &[
    1, 84, 0, 0, 0, 0, 0, 0, 112, 151, 64, 128, 137, 122,
];

/// `Inbound::Act(Intent { ship_id: 42, order: SetCourse { Orbit of Earth, polar, 2 radii,
/// 5 g }, .. })`
///
/// Pinned as well as the other two because a course is the first thing on this wire with a
/// *shape* — nested enums, a string, two floats — rather than a number. It is the message
/// most able to move a field without anyone noticing.
/// `Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: "a.b.c" }`
///
/// Pinned because it is now the message that decides whether anyone gets in at all. A
/// field moving here is a server reading someone else's ticket as this one's.
pub const HELLO: &[u8] = &[
    0, 29, 5, 97, 46, 98, 46, 99,
];

pub const SET_COURSE: &[u8] = &[
    1, 84, 2, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 0, 0, 0, 0,
    20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 128, 137, 122,
];

/// `Inbound::Act(Intent { ship_id: 42, order: Cross { star: 0x0123456789abcdef, 3 g }, .. })`
///
/// Pinned because a star id is the one field on this wire whose bytes nobody can eyeball:
/// it is a hash, so a shifted field reads as a different star rather than as nonsense.
pub const CROSS: &[u8] = &[
    1, 84, 3, 239, 155, 175, 205, 248, 172, 209, 145, 1, 0, 0, 0, 0, 0, 0, 8, 64, 0, 0,
    0, 0, 0, 0, 224, 63, 128, 137, 122,
];

/// `Outbound::Accepted { ship_id: 42, event_id: 9, at_t: 1e6, order: SetCourse { .. 3 g } }`
///
/// Pinned because it is the message a client reconciles against. A field moving here is a
/// client folding the wrong number into where it believes its own ship is.
pub const ACCEPTED: &[u8] = &[
    4, 84, 18, 128, 137, 122, 2, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0, 0, 0, 0, 0, 64,
    1, 0, 0, 0, 0, 0, 0, 8, 64, 0, 0, 0, 0, 0, 0, 208, 63,
];
/// `Outbound::Present([Presence { ship 42 "Ada", 500 m, at [4.2, 0, 0], nose +y }])`
///
/// Pinned because it is the one message that says where somebody *else* is. A field moving
/// here is a client drawing a contact somewhere its light never came from.
pub const PRESENT: &[u8] = &[
    2, 1, 84, 3, 65, 100, 97, 0, 0, 0, 0, 0, 64, 127, 64, 205, 204, 204, 204, 204, 204,
    16, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 252,
    169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 144, 220, 94, 232, 251, 163, 67,
    192, 132, 61, 128, 137, 122,
];

/// `Inbound::Act(Intent { ship 42, Say { to 7, aim Ship(7), sealed, "well?" }, .. })`
///
/// Pinned because it is the first message on this wire carrying text somebody typed, beside
/// two enums that decide who may read it. A field moving between `aim` and `secrecy` is a
/// sealed message released in clear.
///
/// The third byte is the order's discriminant, and it has already earned its keep: merging
/// the refit work moved `Say` from 7 to 9, and this is what said so. The `1` after it is the
/// `Some` of an optional addressee — a broadcast writes a `0` there and nothing else moves.
pub const SAY: &[u8] = &[
    1, 84, 9, 1, 14, 1, 14, 1, 5, 119, 101, 108, 108, 63, 240, 189, 243, 213, 137, 207, 149,
    154, 18, 128, 137, 122,
];

/// `Outbound::Welcome { .., ship: Motion { .., motive: Rendezvous { target: 7, .. } } }`
///
/// Pinned because it is the one motive whose numbers are all about somebody else — a
/// relative offset, a relative velocity, and a sighting. A field moving in it is a pursuer
/// flying at a point its quarry was never at.
/// `Outbound::Welcome { .., ship: Motion { .., motive: Escort { target: 7, .. } } }`
///
/// Pinned beside the rendezvous for the same reason, and one more: its acceleration is the
/// only number on this wire that is a *measurement* of somebody else's burn.
pub const ESCORT: &[u8] = &[
    0, 7, 29, 84, 128, 137, 122, 3, 65, 100, 97, 0, 0, 0, 0, 0, 0, 240, 63, 205, 204,
    204, 204, 204, 204, 16, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24,
    245, 64, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 6, 149, 214, 38, 232, 11,
    46, 17, 190, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 234, 45, 129, 153, 151, 113,
    189, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 119, 43, 65, 0,
    0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0, 56, 156,
    108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 205, 204, 204, 204, 204, 204, 16,
    64, 149, 214, 38, 232, 11, 46, 17, 62, 0, 0, 0, 0, 0, 0, 0, 0, 51, 51, 51, 51, 51,
    51, 211, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 58, 140, 48, 226, 142,
    121, 133, 62, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 112, 111,
    43, 65, 14, 0, 0, 0, 0, 0, 255, 244, 64,
];

pub const RENDEZVOUS: &[u8] = &[
    0, 7, 29, 84, 128, 137, 122, 3, 65, 100, 97, 0, 0, 0, 0, 0, 0, 240, 63, 205, 204,
    204, 204, 204, 204, 16, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24,
    245, 64, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 2, 149, 214, 38, 232, 11,
    46, 17, 62, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    252, 169, 241, 210, 77, 98, 80, 191, 0, 0, 0, 0, 0, 0, 0, 0, 17, 234, 45, 129, 153,
    151, 113, 61, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 119,
    43, 65, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 205, 204, 204, 204, 204,
    204, 16, 64, 149, 214, 38, 232, 11, 46, 17, 62, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    112, 111, 43, 65, 14, 0, 0, 0, 0, 0, 255, 244, 64,
];
/// `Outbound::Welcome { .., ship: Motion { .., motive: Consort { target: 7, .. } } }`
///
/// The rendezvous numbers in a falling frame, pinned for the rendezvous's reason.
pub const CONSORT: &[u8] = &[
    0, 7, 29, 84, 128, 137, 122, 3, 65, 100, 97, 0, 0, 0, 0, 0, 0, 240, 63, 205, 204,
    204, 204, 204, 204, 16, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24,
    245, 64, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 7, 149, 214, 38, 232, 11,
    46, 17, 62, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    252, 169, 241, 210, 77, 98, 80, 191, 0, 0, 0, 0, 0, 0, 0, 0, 17, 234, 45, 129, 153,
    151, 113, 61, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 119,
    43, 65, 0, 0, 0, 0, 0, 0, 20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 0, 0, 0, 0,
    56, 156, 108, 65, 154, 153, 153, 153, 153, 153, 169, 63, 205, 204, 204, 204, 204,
    204, 16, 64, 149, 214, 38, 232, 11, 46, 17, 62, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 252, 169, 241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    112, 111, 43, 65, 14, 0, 0, 0, 0, 0, 255, 244, 64,
];

/// `Inbound::Act(Intent { ship_id: 42, order: Intercept { ship_id: 7, closeness: Intimate }, .. })`
///
/// Pinned because it names a *ship*, and a shifted field is an intercept of whoever the
/// bytes happen to spell.
pub const INTERCEPT: &[u8] = &[
    1, 84, 5, 14, 1, 128, 137, 122,
];

/// `Outbound::Fitted { ship_id: 42, fitting: { starting loadout, a refit to seven engines } }`
///
/// Pinned because it is the account a client previews every refit against. A field moving
/// here is a player told they can afford what they cannot.
pub const FITTED: &[u8] = &[
    11, 84, 0, 0, 0, 0, 0, 0, 240, 63, 102, 102, 102, 102, 102, 102, 238, 63, 0, 0, 0,
    0, 0, 0, 20, 64, 0, 0, 0, 208, 136, 195, 48, 66, 192, 159, 64, 162, 6, 243, 243, 67,
    0, 0, 6, 170, 141, 67, 47, 67, 0, 0, 0, 0, 0, 0, 73, 64, 0, 0, 0, 0, 236, 247, 23,
    65, 205, 204, 204, 204, 204, 188, 120, 64, 102, 102, 102, 102, 102, 102, 230, 63, 0,
    0, 0, 192, 87, 149, 209, 65, 6, 2, 2, 5, 20, 94, 131, 244, 89, 167, 182, 117, 69, 0,
    0, 0, 0, 128, 132, 46, 65, 0, 0, 0, 0, 0, 0, 192, 63, 180, 157, 217, 121, 67, 120,
    234, 68, 0, 200, 78, 103, 109, 193, 139, 67, 1, 6, 2, 2, 5, 20, 6, 2, 2, 7, 20, 94,
    131, 244, 89, 167, 182, 117, 69, 0, 0, 0, 0, 128, 132, 46, 65,
];

/// `Outbound::Library` with one book on the shelf.
pub const LIBRARY: &[u8] = &[
    13, 28, 104, 116, 116, 112, 115, 58, 47, 47, 99, 100, 110, 46, 101, 120, 97, 109, 112,
    108, 101, 47, 108, 105, 98, 114, 97, 114, 121, 47, 1, 14, 116, 104, 101, 45, 103, 105,
    108, 100, 101, 100, 45, 97, 103, 101, 31, 84, 104, 101, 32, 71, 105, 108, 100, 101, 100,
    32, 65, 103, 101, 58, 32, 65, 32, 84, 97, 108, 101, 32, 111, 102, 32, 84, 111, 100, 97,
    121, 1, 10, 77, 97, 114, 107, 32, 84, 119, 97, 105, 110, 1, 11, 84, 119, 97, 105, 110,
    44, 32, 77, 97, 114, 107, 1, 162, 29, 1, 6, 83, 97, 116, 105, 114, 101, 35, 84, 104, 101,
    32, 71, 105, 108, 100, 101, 100, 32, 65, 103, 101, 32, 65, 32, 84, 97, 108, 101, 32, 111,
    102, 32, 84, 111, 100, 97, 121, 46, 101, 112, 117, 98,
];

/// `Outbound::Reading` with one bookmark.
pub const READING: &[u8] =
    &[14, 1, 14, 116, 104, 101, 45, 103, 105, 108, 100, 101, 100, 45, 97, 103, 101, 2, 236, 196, 2, 41, 238, 6];

/// `Inbound::SetReading` with the same bookmark.
pub const SET_READING: &[u8] =
    &[5, 14, 116, 104, 101, 45, 103, 105, 108, 100, 101, 100, 45, 97, 103, 101, 2, 236, 196, 2, 41, 238, 6];

/// `Inbound::Act(Intent { ship_id: 42, order: SendReport { to 7, beamed at 7, open, "{}" }, .. })`
///
/// Pinned because a report is the one thing on this wire carrying a payload a player never
/// reads: a field that moved would fold somebody's survey into the wrong star, and nothing on
/// screen would look wrong.
pub const SEND_REPORT: &[u8] = &[
    1, 84, 11, 1, 14, 1, 14, 0, 2, 123, 125, 161, 134, 149, 187, 152, 245, 242, 246, 15,
    128, 137, 122,
];
