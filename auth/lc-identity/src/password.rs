//! Storing and checking a password, for the one provider that has one.
//!
//! Argon2id with the library's current defaults, and **the whole PHC string stored** —
//! algorithm, parameters and salt in one column. A bare digest cannot be re-tuned without
//! resetting everyone's password; a PHC string can be verified with the parameters it was made
//! with and re-hashed with today's on the next successful sign-in.
//!
//! See `lightcone/docs/16-identity.md` for what is deliberately deferred around this, and what
//! each deferral costs.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{SaltString, rand_core::OsRng};

/// Shortest password accepted.
///
/// Low, and deliberately: with breach-list checking deferred there is nothing here that can
/// tell a good password from a bad one, and a length rule that pretends otherwise is theatre.
/// It exists to catch the empty string and the accidental single keystroke.
pub const MIN_LENGTH: usize = 8;

/// Longest password accepted.
///
/// Argon2 is memory-hard *by parameter*, not by input length, so a very long input is not a
/// cost attack the way it is for bcrypt — but an unbounded one is still a free way to make the
/// server allocate, and nobody's password is a megabyte.
pub const MAX_LENGTH: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Unacceptable {
    TooShort,
    TooLong,
}

/// Hash a new password, salt and all.
pub fn hash(password: &str) -> Result<String, Unacceptable> {
    check(password)?;
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 with a generated salt does not fail")
        .to_string())
}

/// Whether `password` made `phc`.
///
/// A stored string that will not parse comes back `false` rather than an error. There is
/// nothing a caller can usefully do differently, and the one thing it must not do is let
/// someone in.
pub fn verify(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Whether a stored hash was made with parameters weaker than today's.
///
/// The upgrade path: verify with what it was made with, and if this says so, re-hash with what
/// we use now. Only ever called just after a *successful* sign-in, because that is the only
/// moment the plaintext exists.
pub fn needs_rehash(phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return true;
    };
    let salt = SaltString::generate(&mut OsRng);
    let Ok(reference) = Argon2::default().hash_password(b"", &salt) else {
        return false;
    };
    parsed.algorithm != reference.algorithm || parsed.params != reference.params
}

/// The work a sign-in does when the account does not exist.
///
/// Without it, "no account" returns in microseconds and "wrong password" in the tens of
/// milliseconds argon2 costs, and the login form is an account enumerator with a stopwatch.
pub fn waste_time() {
    // A fixed hash of a fixed string: the point is to spend argon2's time, not to be secret.
    const DECOY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
                         ZKq0JRvEYpBLCLGRYOXCQQ0gXCQIbHcqXNxPqpyzJ9Q";
    let _ = verify("no such account", DECOY);
}

fn check(password: &str) -> Result<(), Unacceptable> {
    match password.len() {
        n if n < MIN_LENGTH => Err(Unacceptable::TooShort),
        n if n > MAX_LENGTH => Err(Unacceptable::TooLong),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let phc = hash("correct horse battery").expect("acceptable");
        assert!(verify("correct horse battery", &phc));
        assert!(!verify("correct horse batterz", &phc));
        assert!(!verify("", &phc));
    }

    /// The salt is the point. Two accounts with the same password must not share a hash, or a
    /// stolen table tells you which accounts to attack once.
    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let one = hash("the same password").unwrap();
        let two = hash("the same password").unwrap();
        assert_ne!(one, two, "no salt");
        assert!(verify("the same password", &one) && verify("the same password", &two));
    }

    /// Algorithm, parameters and salt travel with the hash. A column that held only a digest
    /// could not be re-tuned without resetting every password.
    #[test]
    fn the_stored_string_carries_its_own_parameters() {
        let phc = hash("something long enough").unwrap();
        assert!(phc.starts_with("$argon2id$"), "{phc}");
        let parsed = PasswordHash::new(&phc).expect("it parses");
        assert!(parsed.salt.is_some());
        assert!(parsed.params.iter().count() > 0, "no parameters recorded");
        // And today's hash does not want re-hashing, which is what makes the flag meaningful.
        assert!(!needs_rehash(&phc));
    }

    /// Anything unparseable is a failed sign-in, never an accepted one.
    #[test]
    fn a_corrupt_stored_hash_lets_nobody_in() {
        for stored in [
            "",
            "not a hash",
            "$argon2id$",
            "$argon2id$v=19$m=1,t=1,p=1$$",
        ] {
            assert!(
                !verify("anything at all", stored),
                "{stored:?} let someone in"
            );
            assert!(needs_rehash(stored), "{stored:?} should be replaced");
        }
    }

    #[test]
    fn an_empty_or_enormous_password_is_refused() {
        assert_eq!(hash(""), Err(Unacceptable::TooShort));
        assert_eq!(hash("short"), Err(Unacceptable::TooShort));
        assert_eq!(
            hash(&"x".repeat(MAX_LENGTH + 1)),
            Err(Unacceptable::TooLong)
        );
        assert!(hash(&"x".repeat(MAX_LENGTH)).is_ok());
    }

    /// The decoy has to be a *valid* hash, or the dummy verify returns early and costs nothing
    /// — which would leave the timing difference it exists to remove.
    #[test]
    fn the_decoy_actually_costs_what_a_real_verify_costs() {
        let real = hash("a real stored password").unwrap();

        let started = std::time::Instant::now();
        let _ = verify("a wrong guess entirely", &real);
        let against_real = started.elapsed();

        let started = std::time::Instant::now();
        waste_time();
        let against_decoy = started.elapsed();

        // Within an order of magnitude is enough: the failure this guards against is the decoy
        // being parsed as garbage and returning in nanoseconds.
        let ratio = against_real.as_secs_f64() / against_decoy.as_secs_f64().max(1.0e-9);
        assert!(
            ratio < 10.0 && ratio > 0.1,
            "real {against_real:?} against decoy {against_decoy:?}"
        );
    }
}
