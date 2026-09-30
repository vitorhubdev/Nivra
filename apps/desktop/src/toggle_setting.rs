/// Coalesce toggles behind one SQLite write and protect a choice from a late load.
#[derive(Default)]
pub struct Settings {
	pub enabled: bool,
	/// What `enabled` should be before a load completes, and what a load failure falls back
	/// to; an opt-out setting passes `true` here so a missing/failed override still turns on.
	default: bool,
	/// Whether the stored override has loaded (or failed). Observing the startup default
	/// before that would persist a phantom choice and block the real load as "touched".
	loaded: bool,
	pub touched: bool,
	pub dirty: bool,
	pub saving: bool,
	pub failed: bool,
}
impl Settings {
	/// `default` is the value used before the stored override loads, and if loading fails.
	pub fn with_default(default: bool) -> Self {
		Self {
			enabled: default,
			default,
			..Self::default()
		}
	}
	pub fn observe(&mut self, enabled: bool) {
		if !(self.loaded || self.failed) {
			return;
		}
		if self.enabled != enabled {
			self.enabled = enabled;
			self.touched = true;
			self.dirty = true;
			self.failed = false;
		}
	}
	pub fn restore(&mut self, result: Result<bool, local_store::StoreError>) {
		self.loaded = true;
		if !self.touched {
			self.enabled = result.unwrap_or(self.default);
			self.failed = result.is_err();
		}
	}
	pub fn status(&self) -> &'static str {
		if self.failed {
			"Setting could not be saved or loaded. Toggle it to retry saving."
		} else if self.dirty || self.saving {
			"Saving setting…"
		} else {
			""
		}
	}
	pub fn needs_attention(&self) -> bool {
		self.dirty || self.saving || self.failed
	}
}

#[cfg(test)]
mod tests {
	use super::Settings;

	#[test]
	fn startup_default_is_not_a_choice() {
		// Regression: builds between 2026-09-18 and 2026-09-25 observed the
		// startup default before the stored override loaded, persisted a phantom
		// opt-out, and then discarded the real load as "touched".
		let mut setting = Settings::with_default(true);
		setting.observe(false);
		assert!(
			!setting.touched,
			"unloaded default must not count as a choice"
		);
		assert!(!setting.dirty, "unloaded default must not queue a save");
		assert!(setting.enabled, "default stays until the load completes");
	}

	#[test]
	fn restore_then_user_toggle_saves() {
		let mut setting = Settings::with_default(true);
		setting.restore(Ok(false));
		setting.observe(false);
		assert!(!setting.dirty, "restored value is not a change");
		setting.observe(true);
		assert!(setting.dirty, "post-load toggle queues a save");
		assert!(setting.touched);
	}

	#[test]
	fn failed_load_keeps_tracking() {
		let mut setting = Settings {
			failed: true,
			..Settings::with_default(true)
		};
		setting.observe(false);
		assert!(setting.dirty, "without a load there is nothing to protect");
	}
}
