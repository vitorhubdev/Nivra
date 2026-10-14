//! Live WireGuard probe for the owner's provider. Ignored by default: it needs a real endpoint.
//! Run with `NIVRA_WG_CONF=<path to .conf>` and `cargo test -p protect --test live_wireguard -- --ignored --nocapture`.
//! The configuration is read into zeroizing memory and never printed. The probe is the same one
//! as the app's route test: an unauthenticated request plus a UDP DNS round trip, no credentials.
use model::protect::{Draft, Protocol, Scope};
use zeroize::Zeroizing;

#[tokio::test]
#[ignore = "needs a real WireGuard configuration in NIVRA_WG_CONF"]
async fn live_wireguard_route_probe() {
	let path = std::env::var("NIVRA_WG_CONF").expect("set NIVRA_WG_CONF to the .conf path");
	let text = Zeroizing::new(std::fs::read_to_string(path).expect("read configuration"));
	let draft = Draft {
		protocol: Protocol::WireGuard,
		scope: Scope::Complete,
		wireguard: text,
		..Draft::default()
	};
	let profile = protect::prepare(&draft).await.expect("prepare route");
	let outcome = protect::test(profile.clone()).await;
	println!("PROBE: {outcome:?}");
	outcome.expect("route probe");
}
