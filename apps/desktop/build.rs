fn main() {
	build_commit();
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
		windows_icon();
		if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
			// Import-time dependencies resolve from System32 only, so a system-named DLL
			// planted next to Nivra.exe (Online-Fix and friends) is never loaded.
			println!("cargo:rustc-link-arg-bin=nivra=/DEPENDENTLOADFLAG:0x800");
		}
	}
	println!("cargo:rerun-if-changed=../../packaging/macos/Info.plist");
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
		// ScreenCaptureKit's Swift bridge uses the system Swift runtime.
		println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
		// cargo run has no app bundle: embed the same microphone usage description.
		let plist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
			.join("../../packaging/macos/Info.plist");
		println!(
			"cargo:rustc-link-arg-bin=nivra=-Wl,-sectcreate,__TEXT,__info_plist,{}",
			plist.display()
		);
	}
}

fn windows_icon() {
	use std::{path::PathBuf, process::Command};
	println!("cargo:rerun-if-changed=../../packaging/windows/Nivra.ico");
	println!("cargo:rerun-if-changed=../../packaging/windows/nivra.rc");
	let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
		.join("../../packaging/windows");
	let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
	let (mut compiler, resource) = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
	{
		let host = std::env::var("HOST").unwrap();
		let sdk = find_msvc_tools::find_windows_sdk(host.split('-').next().unwrap())
			.expect("Windows SDK is required to embed the application icon");
		let rc = sdk
			.path()
			.map(|path| path.join("rc.exe"))
			.find(|path| path.is_file())
			.expect("Windows SDK rc.exe");
		let resource = out.join("nivra.res");
		let mut compiler = Command::new(rc);
		compiler
			.arg("/nologo")
			.arg("/fo")
			.arg(&resource)
			.arg("nivra.rc");
		(compiler, resource)
	} else {
		let resource = out.join("nivra-icon.o");
		let mut compiler = Command::new("windres");
		compiler.args(["-i", "nivra.rc", "-o"]).arg(&resource);
		(compiler, resource)
	};
	assert!(
		compiler
			.current_dir(root)
			.status()
			.expect("run icon resource compiler")
			.success(),
		"application icon resource compilation failed"
	);
	println!("cargo:rustc-link-arg-bin=nivra={}", resource.display());
}

/// Embeds the commit this binary came from, so a log line can be matched to its source.
/// CI sets GITHUB_SHA; a local build asks git; anything else stays "unknown".
fn build_commit() {
	println!("cargo:rerun-if-env-changed=GITHUB_SHA");
	println!("cargo:rerun-if-changed=../../.git/HEAD");
	let commit = std::env::var("GITHUB_SHA")
		.ok()
		.filter(|sha| sha.len() >= 7)
		.map(|sha| sha[..7].to_owned())
		.or_else(|| {
			std::process::Command::new("git")
				.args(["rev-parse", "--short=7", "HEAD"])
				.output()
				.ok()
				.filter(|out| out.status.success())
				.and_then(|out| String::from_utf8(out.stdout).ok())
				.map(|sha| sha.trim().to_owned())
				.filter(|sha| !sha.is_empty())
		})
		.unwrap_or_else(|| "unknown".to_owned());
	println!("cargo:rustc-env=NIVRA_BUILD_COMMIT={commit}");
}
