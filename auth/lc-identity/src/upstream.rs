//! The OAuth2 *client* half: a redirect out to a provider, and a code exchanged for claims.
//!
//! Authorization code with PKCE, which is the only upstream flow here and the only one worth
//! writing. `lightcone/docs/16-identity.md` has why the downstream half is deliberately not
//! this.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};

use crate::pages::{refusal, sign_in_page};
use crate::providers::{Provider, Upstream};
use crate::routes::{Broker, Destination};
use crate::signin::{self, Refused};
use crate::store::{Flow, Link, Store, normalize_email};

/// How long a player has at the provider's consent screen.
///
/// Long by the standards of everything else here, because the gap is a person reading a page
/// and possibly signing in to Google first. Short enough that an abandoned dance is not a row
/// that lives forever.
pub const FLOW_LIFETIME_S: i64 = 600;

/// The client, with a timeout.
///
/// A token exchange with no deadline is a request handler that can be held open by a provider
/// having a bad day, and enough of those is the broker being down.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("a TLS backend")
}

/// A PKCE pair. The verifier is kept; the challenge is what goes in the URL.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("the system random source");
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the provider sends the browser back to.
///
/// Built from configuration and never from the request. A redirect URI derived from the `Host`
/// header is one an attacker sets, and it is the address a authorization code is delivered to.
pub fn redirect_uri(public_url: &str, provider: Provider) -> String {
    format!("{}/signin/{}/callback", public_url, provider.name())
}

/// The URL that starts the dance.
pub fn authorize_url(
    upstream: &Upstream,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String, Refused> {
    url::Url::parse_with_params(
        &upstream.endpoints.authorize,
        &[
            ("client_id", upstream.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", upstream.endpoints.scope.as_str()),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ],
    )
    .map(String::from)
    .map_err(|why| Refused::Upstream(why.to_string()))
}

/// What the provider says about whoever just signed in.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Claims {
    /// The provider's own identifier. Stable across a name change and an email change, which
    /// is exactly why it and not the address is the `subject` column.
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Whether the **provider** checked the address. Load-bearing: it is the only thing that
    /// may align two accounts. See `lightcone/docs/16-identity.md`.
    #[serde(default)]
    pub email_verified: bool,
    #[serde(default)]
    pub name: Option<String>,
}

impl Claims {
    /// What to call the account. Never the raw address: a display name is shown to other
    /// players, and an email address shown to other players is a leak.
    pub fn display_name(&self) -> String {
        match self.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => "Traveler".to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

/// Swap an authorization code for the provider's claims about a person.
///
/// **The ID token's signature is not checked, and that is the specified behavior rather than a
/// shortcut.** This response arrives over a TLS connection *we* opened to the provider's token
/// endpoint, authenticated with our client secret — there is no path for anyone else to have
/// put a token in it, so a signature proves nothing the transport has not already proven. OIDC
/// Core §3.1.3.7 rule 6 says so directly. Doing it the other way costs a JWKS fetch, a key
/// cache, a rotation story, an outage mode where sign-in fails because a key server is
/// unreachable, and exposure to algorithm confusion — which is *the* JWT bug — for nothing.
///
/// The claims are still checked. `iss` and `aud` are validated against configuration and `exp`
/// against the clock, because "it came from the right socket" is not "it says what it should".
pub async fn exchange_code(
    http: &reqwest::Client,
    upstream: &Upstream,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Claims, Refused> {
    let response = http
        .post(&upstream.endpoints.token)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", upstream.client_id.as_str()),
            ("client_secret", upstream.client_secret.as_str()),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|why| Refused::Upstream(format!("token endpoint unreachable: {why}")))?;

    let status = response.status();
    if !status.is_success() {
        // The body is not repeated anywhere a browser will see it: a provider's error text is
        // for our log, and echoing it into a page is how it becomes a reflection.
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %body.chars().take(400).collect::<String>(), "token endpoint refused");
        return Err(Refused::Upstream(format!("token endpoint said {status}")));
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|why| Refused::Upstream(format!("token response unreadable: {why}")))?;
    claims_from(&token.id_token, upstream)
}

fn validation_for(upstream: &Upstream) -> jsonwebtoken::Validation {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    // See `exchange_code`: the transport is what authenticates this, so the key is unused. The
    // three claim checks below are not.
    validation.insecure_disable_signature_validation();
    validation.set_issuer(&upstream.endpoints.issuers);
    validation.set_audience(&[upstream.client_id.as_str()]);
    validation
}

fn claims_from(id_token: &str, upstream: &Upstream) -> Result<Claims, Refused> {
    jsonwebtoken::decode::<Claims>(
        id_token,
        &jsonwebtoken::DecodingKey::from_secret(b""),
        &validation_for(upstream),
    )
    .map(|data| data.claims)
    .map_err(|why| Refused::Upstream(format!("id token rejected: {why}")))
}

/// Start a dance: a verifier to keep, a state to be handed back, and where to send the browser.
pub async fn begin(
    store: &Store,
    provider: Provider,
    upstream: &Upstream,
    public_url: &str,
    return_to: &str,
    downstream_state: &str,
) -> Result<String, Refused> {
    let pkce = Pkce::new();
    // The same mint the sign-in codes use. A `state` is a nonce that must be unguessable and
    // stored as a digest, which is the whole of what that function makes.
    let (state, digest) = signin::mint_code();
    store
        .put_flow(
            &digest,
            &Flow {
                provider,
                verifier: pkce.verifier,
                return_to: return_to.to_owned(),
                downstream_state: downstream_state.to_owned(),
                expires_at: Utc::now() + ChronoDuration::seconds(FLOW_LIFETIME_S),
            },
        )
        .await?;
    authorize_url(
        upstream,
        &redirect_uri(public_url, provider),
        &state,
        &pkce.challenge,
    )
}

/// Find or make the account a provider's claims name.
///
/// Three outcomes and they are the rule from the design document: a known subject is its own
/// account; an unknown subject whose **verified** address is already spoken for joins that
/// account; anything else is new. An unverified address never aligns, which is what stops a
/// password account registered against someone's Google address from becoming that account.
pub async fn account_from_claims(
    store: &Store,
    provider: Provider,
    claims: &Claims,
) -> Result<Uuid, Refused> {
    if let Some(link) = store.link(provider, &claims.sub).await? {
        return Ok(link.account_id);
    }

    let email = claims.email.as_deref().map(normalize_email);
    let existing = match (&email, claims.email_verified) {
        (Some(email), true) => store.account_for_verified_email(email).await?,
        _ => None,
    };

    let account = store
        .add_link(
            existing,
            &claims.display_name(),
            Link {
                provider,
                subject: claims.sub.clone(),
                account_id: Uuid::nil(),
                email,
                email_verified: claims.email_verified,
            },
        )
        .await?;
    Ok(account.id)
}

/// `GET /signin/{provider}` — leave for the provider.
pub async fn start(
    State(broker): State<Broker>,
    Path(named): Path<String>,
    Query(to): Query<Destination>,
) -> Response {
    let Some((provider, upstream)) = enabled_upstream(&broker, &named) else {
        return refusal(Refused::NoSuchProvider).into_response();
    };
    if let Err(refused) = to.check(&broker.config) {
        return refusal(refused).into_response();
    }
    // `Config::from_env` will not start an upstream provider without one, so this is a
    // constructed-by-hand configuration rather than a deployment.
    let Some(public_url) = broker.config.public_url.as_deref() else {
        return refusal(Refused::Upstream("no public url configured".into())).into_response();
    };

    match begin(
        &broker.store,
        provider,
        upstream,
        public_url,
        &to.return_to,
        &to.state,
    )
    .await
    {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(refused) => {
            tracing::error!(?refused, %provider, "could not start an upstream sign-in");
            refusal(refused).into_response()
        }
    }
}

/// What the provider puts on the query string coming back. All optional, because a provider
/// that is refusing sends `error` and none of the rest.
#[derive(Debug, Deserialize)]
pub struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /signin/{provider}/callback` — come back from the provider, with a code or an excuse.
pub async fn finish(
    State(broker): State<Broker>,
    Path(named): Path<String>,
    Query(back): Query<Callback>,
) -> Response {
    let Some((provider, upstream)) = enabled_upstream(&broker, &named) else {
        return refusal(Refused::NoSuchProvider).into_response();
    };

    // The state is looked up *first*, before the code is even read. It is the only thing tying
    // this request to a sign-in this broker started, and everything below needs what it holds.
    let Some(state) = back.state.as_deref() else {
        return refusal(Refused::NoFlow).into_response();
    };
    let flow = match broker
        .store
        .take_flow(&signin::digest_of(state), Utc::now())
        .await
    {
        Ok(Some(flow)) => flow,
        Ok(None) => return refusal(Refused::NoFlow).into_response(),
        Err(why) => {
            tracing::error!(%why, "could not read a dance in flight");
            return refusal(Refused::Backend(why.to_string())).into_response();
        }
    };
    // A state issued for one provider's dance, presented at another's callback. Nothing good
    // ends that way, and the flow row is already spent.
    if flow.provider != provider {
        return refusal(Refused::NoFlow).into_response();
    }

    // Re-checked, though it was checked when the dance began. The allowlist can have changed
    // in the ten minutes since, and this is the redirect that actually carries the credential.
    let to = Destination {
        return_to: flow.return_to.clone(),
        state: flow.downstream_state.clone(),
    };
    if let Err(refused) = to.check(&broker.config) {
        return refusal(refused).into_response();
    }

    if let Some(error) = &back.error {
        // Ordinary: somebody pressed Cancel. Back to the sign-in page with the destination
        // intact, rather than a dead end they have to navigate out of.
        tracing::info!(%provider, error, "upstream declined");
        return sign_in_page(&broker, &to, Some(&Refused::Declined)).into_response();
    }
    let Some(code) = back.code.as_deref() else {
        return refusal(Refused::NoFlow).into_response();
    };

    let public_url = broker.config.public_url.as_deref().unwrap_or_default();
    let claims = match exchange_code(
        &broker.http,
        upstream,
        &redirect_uri(public_url, provider),
        code,
        &flow.verifier,
    )
    .await
    {
        Ok(claims) => claims,
        Err(refused) => {
            tracing::error!(?refused, %provider, "upstream exchange failed");
            return sign_in_page(&broker, &to, Some(&refused)).into_response();
        }
    };

    let account_id = match account_from_claims(&broker.store, provider, &claims).await {
        Ok(id) => id,
        Err(refused) => {
            tracing::error!(?refused, %provider, "could not resolve an upstream account");
            return sign_in_page(&broker, &to, Some(&refused)).into_response();
        }
    };

    // Including the ban check: an account reached through Google is the same account, and a
    // provider that will happily authenticate somebody is not a provider that gets to decide
    // whether they may play.
    if let Err(refused) = signin::admitted(&broker.store, account_id, Utc::now()).await {
        return refusal(refused).into_response();
    }

    // From here it is the same tail as a password sign-in: one code, spent once, carried back.
    let (code, digest) = signin::mint_code();
    if let Err(why) = broker
        .store
        .put_code(
            &digest,
            account_id,
            &to.return_to,
            Utc::now() + ChronoDuration::seconds(signin::CODE_LIFETIME_S),
        )
        .await
    {
        tracing::error!(%why, "could not record a sign-in code");
        return refusal(Refused::Backend(why.to_string())).into_response();
    }
    Redirect::to(&signin::return_url(&to.return_to, &code, &to.state)).into_response()
}

fn enabled_upstream<'a>(broker: &'a Broker, named: &str) -> Option<(Provider, &'a Upstream)> {
    let provider = Provider::parse(named)?;
    let upstream = broker.config.providers.upstream(provider)?;
    Some((provider, upstream))
}

/// The link that starts a dance, for the sign-in page to render.
pub fn start_url(provider: Provider, to: &Destination) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", &to.return_to)
        .append_pair("state", &to.state)
        .finish();
    format!("/signin/{}?{query}", provider.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google() -> Upstream {
        Upstream {
            client_id: "client-id".into(),
            client_secret: "hunter2".into(),
            endpoints: Provider::Google.endpoints().unwrap(),
        }
    }

    /// Unsigned on purpose: `claims_from` does not look at the signature, so a test that signed
    /// one would be testing a step that is not there. See `exchange_code`.
    fn id_token(claims: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{header}.{payload}.not-a-signature")
    }

    fn google_claims() -> serde_json::Value {
        serde_json::json!({
            "iss": "https://accounts.google.com",
            "aud": "client-id",
            "sub": "1029384756",
            "email": "ada@example.test",
            "email_verified": true,
            "name": "Ada Lovelace",
            "exp": (Utc::now() + ChronoDuration::minutes(5)).timestamp(),
        })
    }

    #[test]
    fn a_verifier_is_its_challenge_hashed() {
        let pkce = Pkce::new();
        assert_eq!(
            pkce.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes())),
        );
        // The length RFC 7636 requires, and unguessable.
        assert!((43..=128).contains(&pkce.verifier.len()));
        assert_ne!(Pkce::new().verifier, pkce.verifier);
    }

    #[test]
    fn the_authorize_url_carries_what_the_provider_needs() {
        let url = authorize_url(
            &google(),
            "https://accounts.lightcone.example/signin/google/callback",
            "the-state",
            "the-challenge",
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], "client-id");
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["code_challenge"], "the-challenge");
        assert_eq!(params["state"], "the-state");
        assert_eq!(
            params["redirect_uri"],
            "https://accounts.lightcone.example/signin/google/callback"
        );
        // The scope OIDC needs to return an email at all.
        assert!(params["scope"].contains("openid") && params["scope"].contains("email"));
        // And the secret is not in a URL a browser follows.
        assert!(!url.contains("hunter2"));
    }

    #[test]
    fn the_redirect_uri_is_what_gets_registered() {
        assert_eq!(
            redirect_uri("https://accounts.lightcone.example", Provider::Google),
            "https://accounts.lightcone.example/signin/google/callback"
        );
    }

    #[test]
    fn claims_are_read_from_the_id_token() {
        let claims = claims_from(&id_token(google_claims()), &google()).unwrap();
        assert_eq!(
            claims,
            Claims {
                sub: "1029384756".into(),
                email: Some("ada@example.test".into()),
                email_verified: true,
                name: Some("Ada Lovelace".into()),
            }
        );
        assert_eq!(claims.display_name(), "Ada Lovelace");
    }

    /// The transport authenticates the token, so these three claims are the whole of what is
    /// left to check — and each is a real attack if it is not.
    ///
    /// Each case names the error it expects. A test that only asserted "an error" would pass
    /// just as well if every one of them failed to parse for some fourth reason, and would
    /// then be proving nothing about the check it is named after.
    #[test]
    fn a_token_for_someone_else_is_refused() {
        use jsonwebtoken::errors::ErrorKind;

        let cases: [(&str, serde_json::Value, ErrorKind); 3] = [
            // Minted for a different client: not ours to accept.
            (
                "aud",
                "some-other-client".into(),
                ErrorKind::InvalidAudience,
            ),
            // Issued by someone else entirely.
            (
                "iss",
                "https://accounts.attacker.test".into(),
                ErrorKind::InvalidIssuer,
            ),
            (
                "exp",
                (Utc::now() - ChronoDuration::hours(1)).timestamp().into(),
                ErrorKind::ExpiredSignature,
            ),
        ];

        for (claim, hostile, expected) in cases {
            let mut token = google_claims();
            token[claim] = hostile;
            let error = jsonwebtoken::decode::<Claims>(
                &id_token(token),
                &jsonwebtoken::DecodingKey::from_secret(b""),
                &validation_for(&google()),
            )
            .expect_err("{claim} was accepted");
            assert_eq!(
                std::mem::discriminant(error.kind()),
                std::mem::discriminant(&expected),
                "{claim} was refused, but for the wrong reason: {error}",
            );
        }
    }

    /// Google issues under two spellings of itself and always has.
    #[test]
    fn either_spelling_of_the_issuer_is_accepted() {
        let mut bare = google_claims();
        bare["iss"] = "accounts.google.com".into();
        assert!(claims_from(&id_token(bare), &google()).is_ok());
    }

    #[test]
    fn an_account_with_no_name_is_still_an_account() {
        let mut nameless = google_claims();
        nameless["name"] = serde_json::Value::Null;
        let claims = claims_from(&id_token(nameless), &google()).unwrap();
        assert_eq!(claims.display_name(), "Traveler");
        // And specifically not the address, which other players would see.
        assert!(!claims.display_name().contains('@'));
    }

    #[tokio::test]
    async fn the_same_subject_signs_in_to_the_same_account() {
        let store = Store::memory();
        let claims = claims_from(&id_token(google_claims()), &google()).unwrap();
        let first = account_from_claims(&store, Provider::Google, &claims)
            .await
            .unwrap();
        let again = account_from_claims(&store, Provider::Google, &claims)
            .await
            .unwrap();
        assert_eq!(first, again, "a second sign-in made a second account");
    }

    /// The alignment rule, in the direction it is meant to work.
    #[tokio::test]
    async fn two_providers_that_both_verified_an_address_are_one_account() {
        let store = Store::memory();
        let google_account = account_from_claims(
            &store,
            Provider::Google,
            &claims_from(&id_token(google_claims()), &google()).unwrap(),
        )
        .await
        .unwrap();

        // The same person, arriving later through something else that also checked the address.
        let elsewhere = Claims {
            sub: "a-different-providers-id".into(),
            email: Some("ADA@Example.test".into()),
            email_verified: true,
            name: Some("Ada".into()),
        };
        let discord_account = account_from_claims(&store, Provider::Discord, &elsewhere)
            .await
            .unwrap();
        assert_eq!(google_account, discord_account);
    }

    /// And in the direction it must not. An unverified address is a string somebody typed.
    #[tokio::test]
    async fn an_unverified_address_aligns_with_nothing() {
        let store = Store::memory();
        // Somebody registers a password account against an address they do not own. Delivery is
        // deferred, so nothing has checked it.
        crate::signin::register_password(
            &store,
            "ada@example.test",
            "a good enough password",
            "Imposter",
        )
        .await
        .unwrap();

        // Ada then arrives through Google, which *has* checked it.
        let account = account_from_claims(
            &store,
            Provider::Google,
            &claims_from(&id_token(google_claims()), &google()).unwrap(),
        )
        .await
        .unwrap();

        let squatter = store
            .link(Provider::Password, "ada@example.test")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            account, squatter.account_id,
            "an unverified address handed over an account"
        );
    }

    /// A provider that says it did not verify is the same as one that said nothing.
    #[tokio::test]
    async fn a_provider_that_did_not_verify_does_not_align() {
        let store = Store::memory();
        let verified = Claims {
            sub: "first".into(),
            email: Some("ada@example.test".into()),
            email_verified: true,
            name: None,
        };
        let unverified = Claims {
            sub: "second".into(),
            email: Some("ada@example.test".into()),
            email_verified: false,
            name: None,
        };
        let one = account_from_claims(&store, Provider::Google, &verified)
            .await
            .unwrap();
        let two = account_from_claims(&store, Provider::Discord, &unverified)
            .await
            .unwrap();
        assert_ne!(one, two);
    }

    #[tokio::test]
    async fn a_dance_is_written_down_and_spent_once() {
        let store = Store::memory();
        let url = begin(
            &store,
            Provider::Google,
            &google(),
            "https://accounts.lightcone.example",
            "https://lightcone.example/auth/return",
            "the-nonce",
        )
        .await
        .unwrap();

        let state = url::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .unwrap();

        let digest = signin::digest_of(&state);
        let flow = store.take_flow(&digest, Utc::now()).await.unwrap().unwrap();
        assert_eq!(flow.return_to, "https://lightcone.example/auth/return");
        assert_eq!(flow.downstream_state, "the-nonce");
        assert_eq!(flow.provider, Provider::Google);

        // Spent. A replayed callback finds nothing.
        assert!(
            store
                .take_flow(&digest, Utc::now())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_abandoned_dance_expires() {
        let store = Store::memory();
        let digest = b"digest".to_vec();
        store
            .put_flow(
                &digest,
                &Flow {
                    provider: Provider::Google,
                    verifier: "v".into(),
                    return_to: "https://lightcone.example/auth/return".into(),
                    downstream_state: "n".into(),
                    expires_at: Utc::now() - ChronoDuration::seconds(1),
                },
            )
            .await
            .unwrap();
        assert!(
            store
                .take_flow(&digest, Utc::now())
                .await
                .unwrap()
                .is_none()
        );
    }
}
