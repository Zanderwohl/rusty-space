//! An account's bans, as the console last stated them.
//!
//! The broker refuses a banned account a ticket, but a connection signed in before the ban
//! presents no ticket again. The console states each change of standing here; the shard drops
//! the account's connections and refuses its sign-ins until the ban ends, so it can neither fly
//! a ship it has nor launch another. Not saved: after a restart every connection signs in again,
//! through the broker.

use lc_proto::{ClientId, Outbound};

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Clear,
    /// Until the Unix second, or for good.
    Banned { until_s: Option<i64> },
}

/// A change of standing, as the console sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stated {
    pub account: String,
    pub standing: Standing,
}

impl<J: Journal> Server<J> {
    pub fn stand(&mut self, stated: Stated, wire: &mut impl Transport) {
        let Stated { account, standing } = stated;
        let Standing::Banned { until_s } = standing else {
            self.banned.remove(&account);
            return;
        };
        self.banned.insert(account.clone(), until_s);
        if !self.is_banned(&account) {
            return;
        }
        let flying = self.clients.iter().filter(|(_, c)| c.account == account).map(|(from, _)| *from);
        let grounded = self.unlaunched.iter().filter(|(_, u)| u.account == account).map(|(from, _)| *from);
        let gone: Vec<ClientId> = flying.chain(grounded).collect();
        for from in gone {
            self.clients.remove(&from);
            self.unlaunched.remove(&from);
            self.owners.retain(|_, owner| *owner != from);
            wire.send(from, Outbound::Unauthenticated);
        }
    }

    /// Forgets a ban once it has run out.
    pub(crate) fn is_banned(&mut self, account: &str) -> bool {
        let Some(&until_s) = self.banned.get(account) else { return false };
        if until_s.is_none_or(|until_s| until_s > wall_s()) {
            return true;
        }
        self.banned.remove(account);
        false
    }
}

/// Unix seconds. A ban's term is wall time, which the shard's coordinate clock is not.
fn wall_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use lc_proto::{Inbound, PROTOCOL_VERSION};

    use super::*;
    use crate::journal::Memory;
    use crate::testing::Broker;
    use crate::transport::Loopback;

    const PILOT: ClientId = ClientId(1);

    fn banned(until_s: Option<i64>) -> Stated {
        Stated { account: "acct-1".into(), standing: Standing::Banned { until_s } }
    }

    async fn signed_in(broker: &Broker, jti: &str) -> (Server<Memory>, Loopback) {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = crate::ticket::Trusted::new("shard-1");
        trusted.learn(&broker.jwks());
        server.trust(trusted);
        let mut wire = Loopback::new();
        wire.client_says(PILOT, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: broker.mint("acct-1", "shard-1", 60, jti) });
        server.tick(&mut wire).await.unwrap();
        (server, wire)
    }

    #[tokio::test]
    async fn a_ban_drops_a_grounded_connection_and_its_launch_is_ignored() {
        let broker = Broker::new([1u8; 32]);
        let (mut server, mut wire) = signed_in(&broker, "j1").await;
        assert_eq!(wire.take(PILOT), vec![Outbound::Unlaunched]);
        server.stand(banned(None), &mut wire);
        assert_eq!(wire.take(PILOT), vec![Outbound::Unauthenticated]);
        wire.client_says(PILOT, crate::testing::launch());
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(PILOT).is_empty());
        assert!(server.fleet.is_empty());
    }

    #[tokio::test]
    async fn a_ban_takes_the_ship_from_its_pilot_and_refuses_the_next_sign_in() {
        let broker = Broker::new([1u8; 32]);
        let (mut server, mut wire) = signed_in(&broker, "j1").await;
        wire.client_says(PILOT, crate::testing::launch());
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(PILOT);
        server.stand(banned(None), &mut wire);
        assert_eq!(wire.take(PILOT), vec![Outbound::Unauthenticated]);
        assert!(server.owned_by(PILOT).is_none() && server.clients.is_empty());

        let again = ClientId(2);
        wire.client_says(again, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: broker.mint("acct-1", "shard-1", 60, "j2") });
        server.tick(&mut wire).await.unwrap();
        assert_eq!(wire.take(again), vec![Outbound::Unauthenticated]);

        server.stand(Stated { account: "acct-1".into(), standing: Standing::Clear }, &mut wire);
        wire.client_says(again, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: broker.mint("acct-1", "shard-1", 60, "j3") });
        server.tick(&mut wire).await.unwrap();
        assert!(matches!(wire.take(again).first(), Some(Outbound::Welcome { .. })));
    }

    #[tokio::test]
    async fn a_ban_that_has_run_out_drops_nobody() {
        let broker = Broker::new([1u8; 32]);
        let (mut server, mut wire) = signed_in(&broker, "j1").await;
        let _ = wire.take(PILOT);
        server.stand(banned(Some(wall_s() - 1)), &mut wire);
        assert!(wire.take(PILOT).is_empty());
        assert!(server.unlaunched.contains_key(&PILOT) && server.banned.is_empty());
    }
}
