//! Ultra-diagnostics lines for PROTECT. Off unless the app turns ultra mode on and registers a sink
//! (the app points it at its bounded, redacted, local-only ultra log). Lines carry counts and error
//! kinds only: never keys, endpoints, addresses or payloads.
use std::sync::{
	OnceLock,
	atomic::{AtomicBool, Ordering},
};

static ULTRA: AtomicBool = AtomicBool::new(false);
static SINK: OnceLock<fn(&str, &str)> = OnceLock::new();

pub fn set_ultra(on: bool) {
	ULTRA.store(on, Ordering::Relaxed);
}

pub fn set_trace_sink(sink: fn(&str, &str)) {
	let _ = SINK.set(sink);
}

pub(crate) fn enabled() -> bool {
	ULTRA.load(Ordering::Relaxed) && SINK.get().is_some()
}

/// One line under the `protect` scope. A no-op outside ultra mode.
pub(crate) fn line(message: &str) {
	if ULTRA.load(Ordering::Relaxed)
		&& let Some(sink) = SINK.get()
	{
		sink("protect", message);
	}
}
