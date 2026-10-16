//! Explicit owner opt-in only. Both fixed HTTPS services go through the tested profile.
use crate::{DEADLINE, Profile};
use std::sync::Arc;

pub struct ExitLocation {
	pub ip: std::net::IpAddr,
	pub country: String,
	pub code: String,
	pub flag_png: Option<Vec<u8>>,
}
async fn bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, &'static str> {
	if !response.status().is_success()
		|| response.content_length().is_some_and(|n| n > limit as u64)
	{
		return Err("Location service rejected the request");
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "Location response failed")?
	{
		if bytes.len().saturating_add(chunk.len()) > limit {
			return Err("Location response exceeded its limit");
		}
		bytes.extend_from_slice(&chunk);
	}
	Ok(bytes)
}
fn parse(bytes: &[u8]) -> Result<ExitLocation, &'static str> {
	let value: serde_json::Value =
		serde_json::from_slice(bytes).map_err(|_| "Invalid location response")?;
	if value.get("success").and_then(|v| v.as_bool()) != Some(true) {
		return Err("Location service could not identify this route");
	}
	let ip = value
		.get("ip")
		.and_then(|v| v.as_str())
		.and_then(|s| s.parse().ok())
		.ok_or("Invalid exit IP")?;
	let country = value
		.get("country")
		.and_then(|v| v.as_str())
		.filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
		.ok_or("Invalid country")?
		.to_owned();
	let code = value
		.get("country_code")
		.and_then(|v| v.as_str())
		.filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()))
		.ok_or("Invalid country code")?
		.to_ascii_lowercase();
	Ok(ExitLocation {
		ip,
		country,
		code,
		flag_png: None,
	})
}
/// Geolocation is approximate; IP is observed via this profile, never the active global route.
/// Failure never sends the request directly and never changes the applied route.
pub async fn exit_location(profile: Arc<Profile>) -> Result<ExitLocation, &'static str> {
	let client = reqwest::Client::builder()
		.no_proxy()
		.proxy(reqwest::Proxy::all(profile.url()?).map_err(|_| "Invalid protected location route")?)
		.redirect(reqwest::redirect::Policy::none())
		.retry(reqwest::retry::never())
		.timeout(DEADLINE)
		.build()
		.map_err(|_| "Location client failed")?;
	let response = client
		.get("https://ipwho.is/?fields=success,ip,country,country_code")
		.send()
		.await
		.map_err(|_| "Location lookup failed through this profile")?;
	let mut location = parse(&bounded(response, 16 * 1024).await?)?;
	// The response cannot choose a URL or path. ISO2 is checked above.
	if let Ok(response) = client
		.get(format!("https://flagcdn.com/32x24/{}.png", location.code))
		.send()
		.await
	{
		location.flag_png = bounded(response, 64 * 1024).await.ok();
	}
	Ok(location)
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn validates_service_success_ip_and_country_before_building_flag_url() {
		let valid =
			br#"{"success":true,"ip":"203.0.113.8","country":"Brazil","country_code":"BR"}"#;
		let location = parse(valid).unwrap();
		assert_eq!(location.code, "br");
		assert_eq!(location.ip.to_string(), "203.0.113.8");
		for invalid in [
			br#"{"success":false}"#.as_slice(),
			br#"{"success":true,"ip":"invalid","country":"Brazil","country_code":"BR"}"#,
			br#"{"success":true,"ip":"203.0.113.8","country":"Brazil","country_code":"../"}"#,
		] {
			assert!(parse(invalid).is_err());
		}
	}
}
