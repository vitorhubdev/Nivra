//! Offline tray-icon proof: renders the five call badges composited by
//! `platform::tray::status_icon` (idle/connecting/connected/muted/deafened). Synthetic
//! evidence for the tray voice-state function; the OS tray itself cannot be
//! captured headlessly.
//! Usage: tray_preview --demo --output-dir=PATH
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	if !args.iter().any(|arg| arg == "--demo") {
		return Err("Usage: tray_preview --demo --output-dir=PATH".into());
	}
	let dir = args
		.iter()
		.find_map(|arg| arg.strip_prefix("--output-dir="))
		.ok_or("Missing --output-dir=PATH")?;
	let png = include_bytes!("../../../packaging/windows/nivra.png");
	for voice in [
		platform::tray::Voice::Idle,
		platform::tray::Voice::Connecting,
		platform::tray::Voice::Connected,
		platform::tray::Voice::Muted,
		platform::tray::Voice::Deafened,
	] {
		let name = format!("{voice:?}.png").to_lowercase();
		let pixels =
			platform::tray::status_icon(png, 64, false, voice).ok_or("tray icon must render")?;
		let path = std::path::Path::new(dir).join(name);
		image::save_buffer(&path, &pixels, 64, 64, image::ColorType::Rgba8)?;
		eprintln!("tray icon: {}", path.display());
	}
	Ok(())
}
