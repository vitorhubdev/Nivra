//! Explicit, scoped emoji and member administration. One request is retained at a time.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{
	Id, permissions as p,
	server_admin::{Action, Emojis, Members, Query, Result as Outcome, Stickers},
};

pub struct Event {
	pub guild: Id,
	pub request: u64,
	pub result: Result<Outcome, Failure>,
}

#[derive(Default)]
pub struct View {
	webhook_url: Option<model::server_integrations::WebhookUrl>,
	pub audit_log: Option<model::server_audit_log::Page>,
	pub audit_query: Option<model::server_audit_log::Query>,
	pub audit_limit_reached: bool,
	pub integrations: Option<model::server_integrations::Snapshot>,
	pub invites: Option<model::server_invites::Snapshot>,
	pub roles: Option<model::server_roles::Catalog>,
	pub selected_role: Option<Id>,
	pub member_role_filter: Option<Id>,
	pub guild: Option<Id>,
	pub emojis: Option<Emojis>,
	pub stickers: Option<Stickers>,
	pub members: Option<Members>,
	pub query: Query,
	pub pending: bool,
	pub saving: bool,
	pub error: Option<&'static str>,
	pub revision: u64,
	pub pruned: Option<u64>,
	pub needs_refresh: bool,
	permission_revision: u64,
	requested_permission_revision: u64,
	sequence: u64,
	action: Option<Action>,
	prune_days: Option<u8>,
}
impl View {
	pub(crate) fn revoke_audit_access(&mut self) {
		self.audit_log = None;
		self.audit_query = None;
		self.audit_limit_reached = false;
		if matches!(self.action, Some(Action::AuditLog(_))) {
			self.reset();
		}
	}
	pub(crate) fn revoke_invite_access(&mut self) {
		self.invites = None;
		if matches!(self.action, Some(Action::Invites(_))) {
			self.reset();
		}
	}
	pub(crate) fn permissions_changed(&mut self, event: &crate::permissions::Event) {
		use crate::permissions::Event;
		let Some(active) = self.guild else {
			return;
		};
		let relevant = match event {
			Event::Snapshot(_) => true,
			Event::Guild(guild) => guild.id == active,
			Event::Role { guild, .. }
			| Event::RoleRemoved { guild, .. }
			| Event::Member { guild, .. }
			| Event::Owner { guild, .. }
			| Event::UnavailableGuild(guild) => *guild == active,
			Event::Members(members) => members.iter().any(|(guild, ..)| *guild == active),
			Event::Channel { guild, .. } => {
				guild.is_none_or(|guild| guild == active)
					&& (self.integrations.is_some()
						|| matches!(self.action, Some(Action::Integrations(_))))
			}
		};
		if relevant {
			self.webhook_url = None;
			self.permission_revision = self.permission_revision.wrapping_add(1);
		}
	}
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence.wrapping_add(1),
			revision: self.revision.wrapping_add(1),
			..Self::default()
		};
	}
}
impl State {
	pub(crate) fn can_retain_channel_integrations(&self, guild: Id) -> bool {
		let scope = match &self.server_admin.action {
			Some(Action::Integrations(action)) => action.scope(),
			_ => self
				.server_admin
				.integrations
				.as_ref()
				.and_then(|page| page.channel),
		};
		scope.is_some_and(|channel| self.can_manage_webhook_channel(guild, channel))
	}
	pub fn can_open_invite_settings(&self, guild: Id) -> bool {
		self.can_manage_guild(guild)
	}
	pub fn can_revoke_guild_invite(&self, guild: Id, code: &str) -> bool {
		self.can_open_invite_settings(guild)
			&& self.server_admin.guild == Some(guild)
			&& model::server_invites::valid_code(code)
			&& self.server_admin.invites.as_ref().is_some_and(|page| {
				page.guild == guild && page.items.iter().any(|invite| invite.code == code)
			})
	}
	pub fn can_pause_guild_invites(&self, guild: Id) -> bool {
		self.can_open_invite_settings(guild)
			&& self.server_admin.guild == Some(guild)
			&& self
				.server_admin
				.invites
				.as_ref()
				.is_some_and(|page| page.guild == guild)
	}
	fn invite_action_allowed(&self, guild: Id, action: &model::server_invites::Action) -> bool {
		match action {
			model::server_invites::Action::Load => self.can_open_invite_settings(guild),
			model::server_invites::Action::Revoke { code } => {
				self.can_revoke_guild_invite(guild, code)
			}
			model::server_invites::Action::SetPaused { .. } => self.can_pause_guild_invites(guild),
		}
	}
	pub fn guild_permission(&self, guild: Id, bits: u128) -> bool {
		let Some(user) = &self.user else {
			return false;
		};
		let Some(metadata) = self.permissions.guilds.get(&guild) else {
			return false;
		};
		self.guild(guild).is_some()
			&& self
				.permissions
				.effective(guild, metadata, user.id, Some(&[]), Self::permission_time())
				.is_some_and(|value| value & bits == bits)
	}
	pub fn can_open_emoji_settings(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::MANAGE_GUILD_EXPRESSIONS)
			|| self.guild_permission(guild, p::CREATE_GUILD_EXPRESSIONS)
	}
	pub fn can_create_guild_emoji(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::CREATE_GUILD_EXPRESSIONS)
	}
	pub fn can_edit_guild_emoji(&self, guild: Id, id: Id) -> bool {
		if self.server_admin.guild != Some(guild) {
			return false;
		}
		let Some(row) = self
			.server_admin
			.emojis
			.as_ref()
			.and_then(|page| page.items.iter().find(|row| row.emoji.id == id))
		else {
			return false;
		};
		!row.emoji.managed
			&& (self.guild_permission(guild, p::MANAGE_GUILD_EXPRESSIONS)
				|| (self.can_create_guild_emoji(guild)
					&& row
						.uploader
						.as_ref()
						.zip(self.user.as_ref())
						.is_some_and(|(uploader, user)| uploader.id == user.id)))
	}
	pub fn can_open_sticker_settings(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::MANAGE_GUILD_EXPRESSIONS)
			|| self.guild_permission(guild, p::CREATE_GUILD_EXPRESSIONS)
	}
	pub fn can_create_guild_sticker(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::CREATE_GUILD_EXPRESSIONS)
	}
	pub fn can_edit_guild_sticker(&self, guild: Id, id: Id) -> bool {
		if self.server_admin.guild != Some(guild) {
			return false;
		}
		let Some(row) = self
			.server_admin
			.stickers
			.as_ref()
			.and_then(|page| page.items.iter().find(|row| row.sticker.id == id))
		else {
			return false;
		};
		self.guild_permission(guild, p::MANAGE_GUILD_EXPRESSIONS)
			|| (self.can_create_guild_sticker(guild)
				&& row
					.uploader
					.as_ref()
					.zip(self.user.as_ref())
					.is_some_and(|(uploader, user)| uploader.id == user.id))
	}
	pub fn can_open_member_settings(&self, guild: Id) -> bool {
		// Discord exposes the Members page to several moderation permissions, not only
		// Manage Guild. Keep this as an any-of gate; action-specific hierarchy checks
		// still decide whether a given member can actually be changed or kicked.
		self.can_manage_guild(guild)
			|| self.guild_permission(guild, p::KICK_MEMBERS)
			|| self.guild_permission(guild, p::BAN_MEMBERS)
			|| self.guild_permission(guild, p::MANAGE_ROLES)
			|| self.guild_permission(guild, p::MANAGE_NICKNAMES)
			|| self.guild_permission(guild, p::MODERATE_MEMBERS)
	}
	fn admin_member(&self, guild: Id, user: Id) -> Option<&model::server_admin::Member> {
		(self.server_admin.guild == Some(guild)).then_some(())?;
		self.server_admin
			.members
			.as_ref()?
			.items
			.iter()
			.find(|member| member.user.id == user)
	}
	fn above_admin_member(&self, guild: Id, user: Id) -> bool {
		let Some(actor) = self.user.as_ref().map(|user| user.id) else {
			return false;
		};
		let Some(metadata) = self.permissions.guilds.get(&guild) else {
			return false;
		};
		let Some(owner) = metadata.owner else {
			return false;
		};
		if user == actor || user == owner {
			return false;
		}
		let Some(member) = self.admin_member(guild, user) else {
			return false;
		};
		if actor == owner {
			return true;
		}
		let Some(roles) = metadata.roles.as_ref() else {
			return false;
		};
		let Some(own) = metadata.member.as_ref() else {
			return false;
		};
		let highest = |ids: &[Id]| -> Option<&p::Role> {
			let everyone = roles.iter().find(|role| role.id == guild)?;
			let mut highest = everyone;
			for id in ids {
				let role = roles.iter().find(|role| role.id == *id)?;
				if role.cmp_hierarchy(highest).is_gt() {
					highest = role;
				}
			}
			Some(highest)
		};
		highest(&own.roles)
			.zip(highest(&member.roles))
			.is_some_and(|(own, target)| own.cmp_hierarchy(target).is_gt())
	}
	pub fn can_kick_guild_member(&self, guild: Id, user: Id) -> bool {
		self.guild_permission(guild, p::KICK_MEMBERS) && self.above_admin_member(guild, user)
	}
	pub fn can_edit_guild_nickname(&self, guild: Id, user: Id) -> bool {
		if self.user.as_ref().is_some_and(|own| own.id == user) {
			self.guild_permission(guild, p::CHANGE_NICKNAME)
		} else {
			self.guild_permission(guild, p::MANAGE_NICKNAMES)
				&& self.above_admin_member(guild, user)
		}
	}
	pub fn can_edit_member_role(&self, guild: Id, user: Id, role: Id) -> bool {
		let own = self.user.as_ref().is_some_and(|actor| actor.id == user);
		if !self.guild_permission(guild, p::MANAGE_ROLES)
			|| (!own && !self.above_admin_member(guild, user))
			|| self.admin_member(guild, user).is_none()
			|| role == guild
		{
			return false;
		}
		let Some(row) = self
			.server_admin
			.members
			.as_ref()
			.and_then(|page| page.roles.iter().find(|row| row.role.id == role))
		else {
			return false;
		};
		if row.managed {
			return false;
		}
		let Some(metadata) = self.permissions.guilds.get(&guild) else {
			return false;
		};
		let Some(current_role) = metadata
			.roles
			.as_ref()
			.and_then(|roles| roles.iter().find(|current| current.id == role))
		else {
			return false;
		};
		if self
			.user
			.as_ref()
			.is_some_and(|user| metadata.owner == Some(user.id))
		{
			return true;
		}
		let Some(own) = metadata.member.as_ref() else {
			return false;
		};
		metadata.roles.as_ref().is_some_and(|roles| {
			roles
				.iter()
				.filter(|role| own.roles.contains(&role.id))
				.any(|role| role.cmp_hierarchy(current_role).is_gt())
		})
	}
	pub fn can_prune_guild(&self, guild: Id) -> bool {
		if self.server_admin.guild != Some(guild) {
			return false;
		}
		let Some(page) = &self.server_admin.members else {
			return false;
		};
		if page
			.features
			.iter()
			.any(|feature| feature == "PRUNE_REQUIRES_ADMIN")
		{
			self.guild_permission(guild, p::ADMINISTRATOR)
		} else {
			self.guild_permission(guild, p::MANAGE_GUILD | p::KICK_MEMBERS)
		}
	}
	pub fn can_show_members_in_channel_list(&self, guild: Id) -> bool {
		self.can_manage_guild(guild)
			&& self.server_admin.guild == Some(guild)
			&& self
				.server_admin
				.members
				.as_ref()
				.is_some_and(|page| page.show_in_channel_list.is_some())
	}
	pub fn server_members_shortcut(&self, guild: Id) -> bool {
		self.can_open_member_settings(guild)
			&& self.server_members_shortcuts.get(&guild) == Some(&true)
	}
	fn voice_move_scope_allowed(&self, guild: Id, from: Id, channel: Id) -> bool {
		if from == channel {
			return false;
		}
		if self
			.channel(from)
			.is_none_or(|source| source.guild != Some(guild) || source.kind != 2)
			|| self
				.channel(channel)
				.is_none_or(|target| target.guild != Some(guild) || target.kind != 2)
		{
			return false;
		}
		self.permission(
			from,
			model::permissions::VIEW_CHANNEL | model::permissions::MOVE_MEMBERS,
		) == Some(true)
			&& self.permission(
				channel,
				model::permissions::VIEW_CHANNEL | model::permissions::MOVE_MEMBERS,
			) == Some(true)
	}
	/// Whether this exact roster entry can be dragged right now. Destination permission is
	/// checked separately while hovering, so invalid channels simply become non-drop targets.
	pub fn can_drag_voice_member(&self, guild: Id, user: Id, from: Id) -> bool {
		user.0 != 0
			&& !self.server_admin.pending
			&& !self.server_settings.saving
			&& !self.server_admin.needs_refresh
			&& (self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected))
			&& self
				.channel(from)
				.is_some_and(|channel| channel.guild == Some(guild) && channel.kind == 2)
			&& self.permission(
				from,
				model::permissions::VIEW_CHANNEL | model::permissions::MOVE_MEMBERS,
			) == Some(true)
			&& self.voice.roster.iter().any(|entry| {
				entry.guild == guild && entry.channel == from && entry.participant.user == user
			})
	}
	/// Permission and freshness fence used both when dropping and immediately before the
	/// REST write. The voice roster is authoritative for the source channel.
	pub fn can_move_voice_member(&self, guild: Id, user: Id, from: Id, channel: Id) -> bool {
		user.0 != 0
			&& (self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected))
			&& self.voice_move_scope_allowed(guild, from, channel)
			&& self.voice.roster.iter().any(|entry| {
				entry.guild == guild && entry.channel == from && entry.participant.user == user
			})
	}

	pub fn server_admin_action_allowed(&self, guild: Id, action: &Action) -> bool {
		if !action.valid() {
			return false;
		}
		match action {
			Action::AuditLog(query) => self.audit_log_action_allowed(guild, query),
			Action::Roles(action) => self.role_action_allowed(guild, action),
			Action::Invites(action) => self.invite_action_allowed(guild, action),
			Action::Integrations(action) => self.integration_action_allowed(guild, action),
			Action::LoadEmojis => self.can_open_emoji_settings(guild),
			Action::CreateEmoji { .. } => self.can_create_guild_emoji(guild),
			Action::RenameEmoji { id, .. } | Action::DeleteEmoji { id } => {
				self.can_edit_guild_emoji(guild, *id)
			}
			Action::LoadStickers => self.can_open_sticker_settings(guild),
			Action::CreateSticker { .. } => self.can_create_guild_sticker(guild),
			Action::EditSticker { id, .. } | Action::DeleteSticker { id } => {
				self.can_edit_guild_sticker(guild, *id)
			}
			Action::LoadMembers(_) => self.can_open_member_settings(guild),
			Action::SetRole { user, role, .. } => self.can_edit_member_role(guild, *user, *role),
			Action::SetNickname { user, .. } => self.can_edit_guild_nickname(guild, *user),
			Action::MoveVoice {
				user,
				from,
				channel,
			} => self.can_move_voice_member(guild, *user, *from, *channel),
			Action::Kick { user } => self.can_kick_guild_member(guild, *user),
			Action::Prune { days, execute } => {
				self.can_prune_guild(guild)
					&& (!execute
						|| (self.server_admin.prune_days == Some(*days)
							&& self.server_admin.pruned.is_some()))
			}
			Action::ShowMembers { .. } => self.can_show_members_in_channel_list(guild),
		}
	}
	pub fn request_server_admin(&mut self, guild: Id, mut action: Action) -> Option<Command> {
		match &mut action {
			Action::Roles(action) => action.normalize(),
			Action::Invites(action) => action.normalize(),
			Action::Integrations(action) => action.normalize(),
			Action::LoadMembers(query) => query.search.shrink_to_fit(),
			Action::SetNickname { nick, .. } => nick.shrink_to_fit(),
			Action::CreateEmoji { name, image } => {
				name.shrink_to_fit();
				image.shrink_to_fit();
			}
			Action::RenameEmoji { name, .. } => name.shrink_to_fit(),
			Action::CreateSticker {
				name,
				description,
				tags,
				filename,
				content_type,
				file,
			} => {
				name.shrink_to_fit();
				description.shrink_to_fit();
				tags.shrink_to_fit();
				filename.shrink_to_fit();
				content_type.shrink_to_fit();
				file.shrink_to_fit();
			}
			Action::EditSticker {
				name,
				description,
				tags,
				..
			} => {
				name.shrink_to_fit();
				description.shrink_to_fit();
				tags.shrink_to_fit();
			}
			_ => {}
		}
		if self.server_admin.pending
			|| self.server_settings.saving
			|| (self.server_admin.needs_refresh && action.write())
			|| !self.server_admin_action_allowed(guild, &action)
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		if self.server_admin.guild != Some(guild) {
			self.server_admin.reset();
		}
		if let Action::AuditLog(query) = &action {
			if query.before.is_none() {
				self.server_admin.audit_log = None;
				self.server_admin.audit_limit_reached = false;
			}
			self.server_admin.audit_query = Some(query.clone());
		}
		if let Action::LoadMembers(query) = &action {
			self.server_admin.query = query.clone();
			self.server_admin.member_role_filter = None;
		} else if let Action::Roles(model::server_roles::Action::Members { role, query }) = &action
		{
			self.server_admin.query = query.clone();
			self.server_admin.member_role_filter = *role;
		}
		self.server_admin.webhook_url = None;
		self.server_admin.guild = Some(guild);
		self.server_admin.sequence = self.server_admin.sequence.wrapping_add(1);
		self.server_admin.pending = true;
		self.server_admin.requested_permission_revision = self.server_admin.permission_revision;
		self.server_admin.saving = action.write();
		self.server_admin.error = None;
		let mut retained = action.clone();
		if let Action::CreateEmoji { image, .. } = &mut retained {
			*image = String::new();
		} else if let Action::CreateSticker { file, .. } = &mut retained {
			file.clear();
			file.shrink_to_fit();
		} else if let Action::Roles(
			model::server_roles::Action::Create(edit)
			| model::server_roles::Action::Edit { edit, .. },
		) = &mut retained
		{
			edit.icon = model::Patch::Absent;
		}
		self.server_admin.action = Some(retained);
		Some(Command::ServerAdmin {
			guild,
			request: self.server_admin.sequence,
			action: Box::new(action),
		})
	}
	pub fn server_admin_command_allowed(&self, guild: Id, request: u64, action: &Action) -> bool {
		self.server_admin.guild == Some(guild)
			&& self.server_admin.sequence == request
			&& self.server_admin.pending
			&& self.server_admin_action_allowed(guild, action)
	}
	/// Consume a ready URL only while the same permission-scoped settings remain open.
	pub fn take_webhook_url(
		&mut self,
		guild: Id,
		scope: Option<Id>,
	) -> Option<model::server_integrations::WebhookUrl> {
		let url = self.server_admin.webhook_url.take()?;
		let action = model::server_integrations::Action::CopyWebhookUrl {
			scope,
			webhook: url.webhook,
			channel: url.channel,
		};
		(url.guild == guild
			&& (self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected))
			&& self.integration_action_allowed(guild, &action))
		.then_some(url)
	}
	/// Fence an in-flight copy without discarding the integration metadata.
	pub fn clear_webhook_url(&mut self) {
		self.server_admin.webhook_url = None;
		if matches!(
			self.server_admin.action,
			Some(Action::Integrations(
				model::server_integrations::Action::CopyWebhookUrl { .. }
			))
		) {
			self.server_admin.action = None;
			self.server_admin.pending = false;
			self.server_admin.sequence = self.server_admin.sequence.wrapping_add(1);
		}
	}
	pub fn close_server_admin(&mut self) {
		if !self.server_admin.saving {
			self.server_admin.reset();
		}
	}
	pub(crate) fn cancel_server_admin(&mut self) {
		self.server_admin.webhook_url = None;
		if self.server_admin.pending {
			self.server_admin.needs_refresh |= self.server_admin.saving;
			self.server_admin.error = Some(if self.server_admin.saving {
				"Outcome unknown; reload before retrying"
			} else {
				"Server administration disconnected; reload to continue"
			});
			self.server_admin.pending = false;
			self.server_admin.saving = false;
			self.server_admin.action = None;
			self.server_admin.sequence = self.server_admin.sequence.wrapping_add(1);
		}
	}
	pub(crate) fn apply_server_admin(&mut self, event: Event) -> Result<(), &'static str> {
		if self.server_admin.guild != Some(event.guild)
			|| self.server_admin.sequence != event.request
			|| !self.server_admin.pending
		{
			return Ok(());
		}
		let action = self.server_admin.action.take();
		self.server_admin.pending = false;
		self.server_admin.saving = false;
		if action.as_ref().is_none_or(|action| {
			if matches!(action, Action::AuditLog(_)) {
				!self.can_open_audit_log_settings(event.guild)
			} else if let Action::Integrations(action) = action {
				!self.integration_action_allowed(event.guild, action)
			} else if matches!(action, Action::Invites(_)) {
				!self.can_open_invite_settings(event.guild)
			} else if matches!(action, Action::Roles(_)) {
				!self.can_open_role_settings(event.guild)
					|| matches!(
						action,
						Action::Roles(model::server_roles::Action::Members { .. })
					) && !self.can_open_member_settings(event.guild)
			} else if action.emoji() {
				!self.can_open_emoji_settings(event.guild)
			} else if let Action::MoveVoice { from, channel, .. } = action {
				!self.voice_move_scope_allowed(event.guild, *from, *channel)
			} else if action.sticker() {
				!match action {
					Action::LoadStickers => self.can_open_sticker_settings(event.guild),
					Action::CreateSticker { .. } => self.can_create_guild_sticker(event.guild),
					Action::EditSticker { id, .. } | Action::DeleteSticker { id } => {
						self.can_edit_guild_sticker(event.guild, *id)
					}
					_ => false,
				}
			} else {
				!self.can_open_member_settings(event.guild)
			}
		}) {
			self.server_admin.reset();
			return Ok(());
		}
		let result = match event.result {
			Ok(result) if result.valid() => result,
			Ok(_) => {
				self.server_admin.error =
					Some("Server administration response exceeded safe bounds");
				self.server_admin.needs_refresh = true;
				return Ok(());
			}
			Err(failure) => {
				if matches!(action, Some(Action::AuditLog(_))) && failure == Failure::Forbidden {
					self.server_admin.revoke_audit_access();
				}
				if matches!(action, Some(Action::MoveVoice { .. })) {
					self.status = failure.label();
				}
				self.server_admin.error = Some(failure.label());
				self.server_admin.needs_refresh |= action.as_ref().is_some_and(Action::write)
					&& !matches!(action, Some(Action::MoveVoice { .. }))
					&& failure == Failure::Ambiguous;
				if failure.ends_session() {
					self.fail(failure);
				}
				return Ok(());
			}
		};
		if matches!(action, Some(Action::AuditLog(_)))
			&& self.server_admin.requested_permission_revision
				!= self.server_admin.permission_revision
		{
			self.server_admin.audit_log = None;
			self.server_admin.error = Some("Server permissions changed; reload the audit log");
			return Ok(());
		}
		if matches!(action, Some(Action::Integrations(_)))
			&& self.server_admin.requested_permission_revision
				!= self.server_admin.permission_revision
		{
			self.server_admin.integrations = None;
			self.server_admin.error =
				Some("Server permissions changed; reload integrations to continue");
			self.server_admin.needs_refresh = true;
			return Ok(());
		}
		if matches!(action, Some(Action::Roles(_)))
			&& self.server_admin.requested_permission_revision
				!= self.server_admin.permission_revision
			&& !matches!(&result, Outcome::Roles(model::server_roles::Result::Catalog { catalog, .. }) if self.roles_catalog_matches_permissions(catalog))
		{
			self.server_admin.error =
				Some("Server roles or permissions changed; reload before continuing");
			self.server_admin.needs_refresh = true;
			return Ok(());
		}
		let expected = matches!(
			(&action, &result),
			(
				Some(
					Action::LoadEmojis
						| Action::CreateEmoji { .. }
						| Action::RenameEmoji { .. }
						| Action::DeleteEmoji { .. }
				),
				Outcome::Emojis(_)
			) | (Some(Action::LoadMembers(_)), Outcome::Members(_))
				| (Some(Action::Prune { .. }), Outcome::Pruned(_))
				| (Some(Action::ShowMembers { .. }), Outcome::ChannelList(_))
		) || match (&action, &result) {
			(
				Some(Action::Integrations(model::server_integrations::Action::CopyWebhookUrl {
					webhook,
					channel,
					..
				})),
				Outcome::WebhookUrl(url),
			) => url.guild == event.guild && url.webhook == *webhook && url.channel == *channel,
			(Some(Action::AuditLog(query)), Outcome::AuditLog(page)) => {
				page.guild == event.guild && page.matches_query(query)
			}
			(Some(Action::Integrations(action)), Outcome::Integrations(snapshot)) => {
				snapshot.guild == event.guild
					&& crate::server_integrations::expected(action, snapshot)
			}
			(Some(Action::Invites(action)), Outcome::Invites(snapshot)) => {
				snapshot.guild == event.guild
					&& match action {
						model::server_invites::Action::Load => true,
						model::server_invites::Action::Revoke { code } => {
							snapshot.items.iter().all(|invite| invite.code != *code)
						}
						model::server_invites::Action::SetPaused { paused } => {
							snapshot.paused() == *paused
						}
					}
			}
			(Some(Action::Roles(action)), Outcome::Roles(result)) => match (action, result) {
				(
					model::server_roles::Action::Members { role, .. },
					model::server_roles::Result::Members {
						role: returned,
						page,
					},
				) => {
					role == returned
						&& role.is_none_or(|role| {
							role == event.guild
								|| page.items.iter().all(|member| member.roles.contains(&role))
						})
				}
				(
					model::server_roles::Action::Load
					| model::server_roles::Action::Create(_)
					| model::server_roles::Action::Edit { .. }
					| model::server_roles::Action::Delete(_)
					| model::server_roles::Action::Move { .. },
					model::server_roles::Result::Catalog { catalog, .. },
				) => catalog.guild == event.guild,
				_ => false,
			},
			(
				Some(Action::SetRole { user, .. } | Action::SetNickname { user, .. }),
				Outcome::Member(member),
			) => *user == member.user.id,
			(
				Some(Action::MoveVoice { user, channel, .. }),
				Outcome::VoiceMoved {
					user: moved,
					channel: destination,
				},
			) => user == moved && channel == destination,
			(Some(Action::Kick { user }), Outcome::Kicked(id)) => user == id,
			(Some(action), Outcome::Stickers(page)) if action.sticker() => {
				page.items
					.iter()
					.all(|row| row.sticker.guild_id == Some(event.guild))
					&& match action {
						Action::LoadStickers => true,
						Action::CreateSticker {
							name,
							description,
							tags,
							..
						} => page.items.iter().any(|row| {
							row.sticker.name == *name
								&& row.sticker.description == *description
								&& row.sticker.tags == *tags
						}),
						Action::EditSticker {
							id,
							name,
							description,
							tags,
						} => page.items.iter().any(|row| {
							row.sticker.id == *id
								&& row.sticker.name == *name
								&& row.sticker.description == *description
								&& row.sticker.tags == *tags
						}),
						Action::DeleteSticker { id } => {
							page.items.iter().all(|row| row.sticker.id != *id)
						}
						_ => false,
					}
			}
			_ => false,
		};
		if !expected {
			self.server_admin.error =
				Some("Unexpected server administration response; reload to continue");
			self.server_admin.needs_refresh = true;
			return Ok(());
		}
		match result {
			Outcome::WebhookUrl(url) => self.server_admin.webhook_url = Some(url),
			Outcome::AuditLog(page) => self.apply_audit_log(page),
			Outcome::Integrations(snapshot) => {
				if let Some(Action::Integrations(action)) = &action {
					self.apply_integrations(snapshot, action);
				}
			}
			Outcome::Invites(snapshot) => self.server_admin.invites = Some(snapshot),
			Outcome::Roles(result) => match result {
				model::server_roles::Result::Catalog { catalog, selected } => {
					self.apply_roles_catalog(catalog, selected)
				}
				model::server_roles::Result::Members { role, page } => {
					self.server_admin.member_role_filter = role;
					self.server_admin.members = Some(page);
				}
			},
			Outcome::Emojis(page) => {
				self.apply(crate::Envelope {
					generation: self.generation,
					event: crate::Event::GuildEmojis {
						guild: event.guild,
						emojis: page.items.iter().map(|row| row.emoji.clone()).collect(),
					},
				});
				self.server_admin.emojis = Some(page);
			}
			Outcome::Stickers(page) => {
				self.apply(crate::Envelope {
					generation: self.generation,
					event: crate::Event::GuildStickers {
						guild: event.guild,
						stickers: page.items.iter().map(|row| row.sticker.clone()).collect(),
					},
				});
				self.server_admin.stickers = Some(page);
			}
			Outcome::Members(page) => {
				if let Some(enabled) = page.show_in_channel_list {
					if self.server_members_shortcuts.len() < 4000
						|| self.server_members_shortcuts.contains_key(&event.guild)
					{
						self.server_members_shortcuts.insert(event.guild, enabled);
					}
				} else {
					self.server_members_shortcuts.remove(&event.guild);
				}
				self.server_admin.members = Some(page);
			}
			Outcome::Member(mut member) => {
				if matches!(action, Some(Action::SetRole { .. }))
					&& self
						.user
						.as_ref()
						.is_some_and(|own| own.id == member.user.id)
					&& self.server_admin.requested_permission_revision
						== self.server_admin.permission_revision
				{
					if self
						.update_permissions(crate::permissions::Event::Member {
							guild: event.guild,
							roles: model::Patch::Value(member.roles.clone()),
							timeout_until: model::Patch::Absent,
						})
						.is_err()
					{
						self.fail(crate::auth::Failure::Capacity);
						return Err("Permission metadata exceeds safe capacity");
					}
					self.permissions.clear_cache();
					self.invalidate_navigation();
				}
				if let Some(Action::SetRole { role, assigned, .. }) = &action {
					if let Some(row) =
						self.server_admin.roles.as_mut().and_then(|catalog| {
							catalog.items.iter_mut().find(|row| row.id == *role)
						}) {
						row.member_count = None;
					}
					if self.server_admin.member_role_filter == Some(*role)
						&& !assigned && let Some(page) = &mut self.server_admin.members
					{
						let before = page.items.len();
						page.items.retain(|row| row.user.id != member.user.id);
						page.total = page
							.total
							.saturating_sub((before - page.items.len()) as u64);
					}
				}
				if let Some(row) = self.server_admin.members.as_mut().and_then(|page| {
					page.items
						.iter_mut()
						.find(|row| row.user.id == member.user.id)
				}) {
					member.join_source = row.join_source;
					member.invite_code.clone_from(&row.invite_code);
					*row = member;
				}
			}
			// The gateway VOICE_STATE_UPDATE remains authoritative for roster placement.
			Outcome::VoiceMoved { .. } => {
				self.status = "Voice participant moved";
			}
			Outcome::Kicked(user) => {
				if let Some(page) = &mut self.server_admin.members {
					page.items.retain(|row| row.user.id != user);
					page.total = page.total.saturating_sub(1);
				}
			}
			Outcome::Pruned(count) => {
				self.server_admin.pruned = count;
				self.server_admin.prune_days = match &action {
					Some(Action::Prune {
						days,
						execute: false,
					}) => Some(*days),
					_ => None,
				};
			}
			Outcome::ChannelList(enabled) => {
				if let Some(page) = &mut self.server_admin.members {
					page.show_in_channel_list = Some(enabled);
				}
				if self.server_members_shortcuts.len() < 4000
					|| self.server_members_shortcuts.contains_key(&event.guild)
				{
					self.server_members_shortcuts.insert(event.guild, enabled);
				}
			}
		}
		if matches!(
			action,
			Some(
				Action::LoadEmojis
					| Action::LoadStickers
					| Action::Invites(model::server_invites::Action::Load)
					| Action::Integrations(model::server_integrations::Action::Load { .. })
					| Action::LoadMembers(_)
					| Action::Roles(model::server_roles::Action::Load)
			)
		) {
			self.server_admin.needs_refresh = false;
		}
		self.server_admin.revision = self.server_admin.revision.wrapping_add(1);
		Ok(())
	}
}

#[cfg(test)]
mod member_kick_tests {
	use super::*;
	use model::permissions as p;

	fn user(id: u64, name: &str) -> model::User {
		model::User {
			id: Id(id),
			name: name.into(),
			avatar: None,
			discriminator: 0,
			kind: Default::default(),
			webhook: false,
			primary_guild: None,
		}
	}
	fn role(id: u64, bits: u128, position: i32) -> p::Role {
		p::Role {
			id: Id(id),
			name: format!("role-{id}"),
			bits,
			color: 0,
			position,
			hoist: false,
		}
	}
	fn admin_member(id: u64, roles: Vec<Id>) -> model::server_admin::Member {
		model::server_admin::Member {
			user: user(id, &format!("user-{id}")),
			nick: None,
			roles,
			joined_at: None,
			join_source: None,
			invite_code: None,
			flags: None,
			unusual_dm_until: None,
			timeout_until: None,
		}
	}
	fn state(owner_actor: bool, administrator: bool) -> State {
		let guild = Id(10);
		let actor = Id(1);
		let owner = if owner_actor { actor } else { Id(99) };
		let actor_role = Id(20);
		let lower = Id(30);
		let older_peer = Id(19);
		let newer_peer = Id(31);
		let higher = Id(32);
		let actor_bits = if administrator {
			p::ADMINISTRATOR
		} else {
			p::KICK_MEMBERS
		};
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(user(actor.0, "Moderator")),
			guilds: vec![model::Guild {
				id: guild,
				name: "Guild".into(),
				icon: None,
				emojis: None,
				stickers: None,
				premium_tier: 0,
			}],
			..Default::default()
		};
		state.permissions.guilds.insert(
			guild,
			p::Guild {
				id: guild,
				owner: Some(owner),
				member: Some(p::Member {
					roles: (!owner_actor)
						.then_some(vec![actor_role])
						.unwrap_or_default(),
					timeout_until: None,
				}),
				roles: Some(vec![
					role(guild.0, 0, 0),
					role(actor_role.0, actor_bits, 5),
					role(lower.0, 0, 1),
					role(older_peer.0, 0, 5),
					role(newer_peer.0, 0, 5),
					role(higher.0, 0, 10),
				]),
			},
		);
		state.server_admin.guild = Some(guild);
		state.server_admin.members = Some(model::server_admin::Members {
			items: vec![
				admin_member(2, vec![lower]),
				admin_member(3, vec![older_peer]),
				admin_member(5, vec![newer_peer]),
				admin_member(4, vec![higher]),
				admin_member(
					actor.0,
					if owner_actor {
						vec![]
					} else {
						vec![actor_role]
					},
				),
			],
			total: 5,
			..Default::default()
		});
		state
	}

	#[test]
	fn kick_permissions_respect_owner_and_role_hierarchy() {
		let guild = Id(10);

		let owner = state(true, false);
		assert!(owner.can_kick_guild_member(guild, Id(2)));
		assert!(owner.can_kick_guild_member(guild, Id(3)));
		assert!(owner.can_kick_guild_member(guild, Id(5)));
		assert!(owner.can_kick_guild_member(guild, Id(4)));
		assert!(!owner.can_kick_guild_member(guild, Id(1)));
		assert!(!owner.can_kick_guild_member(guild, Id(99)));

		for administrator in [false, true] {
			let mut moderator = state(false, administrator);
			assert!(moderator.can_open_member_settings(guild));
			assert!(moderator.can_kick_guild_member(guild, Id(2)));
			// Same position is not a tie: the older role ID ranks above.
			assert!(!moderator.can_kick_guild_member(guild, Id(3)));
			assert!(moderator.can_kick_guild_member(guild, Id(5)));
			assert!(!moderator.can_kick_guild_member(guild, Id(4)));
			assert!(!moderator.can_kick_guild_member(guild, Id(1)));
			assert!(!moderator.can_kick_guild_member(guild, Id(99)));

			if !administrator {
				let roles = moderator
					.permissions
					.guilds
					.get_mut(&guild)
					.unwrap()
					.roles
					.as_mut()
					.unwrap();
				roles
					.iter_mut()
					.find(|role| role.id == Id(20))
					.unwrap()
					.bits = 0;
				moderator.permissions.clear_cache();
				assert!(!moderator.can_kick_guild_member(guild, Id(2)));
			}
		}
	}
}

#[cfg(test)]
mod voice_move_tests {
	use super::*;
	use crate::voice::{Participant, RosterEntry};
	use model::permissions as p;

	fn state() -> State {
		let guild = Id(10);
		let source = Id(11);
		let target = Id(12);
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(1),
				name: "Moderator".into(),
				avatar: None,
				discriminator: 0,
				kind: Default::default(),
				webhook: false,
				primary_guild: None,
			}),
			guilds: vec![model::Guild {
				id: guild,
				name: "Guild".into(),
				icon: None,
				emojis: None,
				stickers: None,
				premium_tier: 0,
			}],
			channels: [source, target]
				.into_iter()
				.enumerate()
				.map(|(position, id)| model::Channel {
					id,
					guild: Some(guild),
					parent_id: None,
					position: position as i32,
					name: format!("Voice {position}"),
					kind: 2,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
					last_message: None,
					tags: None,
				})
				.collect(),
			..Default::default()
		};
		state.permissions.guilds.insert(
			guild,
			p::Guild {
				id: guild,
				owner: Some(Id(99)),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
				roles: Some(vec![p::Role {
					id: guild,
					name: "@everyone".into(),
					bits: p::VIEW_CHANNEL | p::MOVE_MEMBERS,
					color: 0,
					position: 0,
					hoist: false,
				}]),
			},
		);
		for id in [source, target] {
			state.permissions.channels.insert(
				id,
				p::Channel {
					id,
					guild,
					overwrites: Some(vec![]),
				},
			);
		}
		state.voice.roster.push(RosterEntry {
			guild,
			channel: source,
			participant: Participant {
				user: Id(8),
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
			member: None,
		});
		state
	}

	#[test]
	fn voice_member_move_requires_roster_and_move_members_on_both_channels() {
		let mut state = state();
		assert!(state.can_drag_voice_member(Id(10), Id(8), Id(11)));
		assert!(state.can_move_voice_member(Id(10), Id(8), Id(11), Id(12)));
		assert!(!state.can_move_voice_member(Id(10), Id(8), Id(11), Id(11)));
		assert!(!state.can_move_voice_member(Id(10), Id(9), Id(11), Id(12)));

		state.permissions.channels.insert(
			Id(12),
			p::Channel {
				id: Id(12),
				guild: Id(10),
				overwrites: Some(vec![p::Overwrite {
					id: Id(10),
					kind: 0,
					allow: 0,
					deny: p::MOVE_MEMBERS,
				}]),
			},
		);
		state.permissions.clear_cache();
		assert!(!state.can_move_voice_member(Id(10), Id(8), Id(11), Id(12)));
	}
}

#[cfg(test)]
mod invite_tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	use model::server_invites::{Action as InviteAction, Invite, Snapshot};
	fn page() -> Snapshot {
		Snapshot {
			guild: Id(2),
			items: vec![Invite {
				code: "synthetic_code".into(),
				inviter: None,
				channel: Some(Id(3)),
				channel_name: Some("chat".into()),
				uses: Some(0),
				max_uses: Some(0),
				max_age: Some(0),
				created_at: None,
				expires_at: None,
				temporary: Some(false),
				roles: None,
			}],
			features: vec!["FUTURE_FEATURE".into()],
		}
	}
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				primary_guild: None,
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				discriminator: 0,
				kind: Default::default(),
				webhook: false,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				premium_tier: 0,
			}],
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			p::Guild {
				id: Id(2),
				owner: Some(Id(99)),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
				roles: Some(vec![p::Role {
					id: Id(2),
					name: "@everyone".into(),
					bits: p::MANAGE_GUILD | p::MANAGE_ROLES,
					color: 0,
					position: 0,
					hoist: false,
				}]),
			},
		);
		state.server_admin.guild = Some(Id(2));
		state.server_admin.invites = Some(page());
		state
	}
	fn deliver(state: &mut State, guild: Id, request: u64, result: Result<Outcome, Failure>) {
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ServerAdmin(Event {
				guild,
				request,
				result,
			}),
		});
	}
	#[test]
	fn invites_stale_requests_and_permission_loss_do_not_restore_codes() {
		let mut state = state();
		assert!(!state.can_revoke_guild_invite(Id(2), "unknown"));
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(Id(2), Action::Invites(InviteAction::Load))
			.unwrap()
		else {
			panic!()
		};
		deliver(&mut state, guild, request + 1, Ok(Outcome::Invites(page())));
		assert!(state.server_admin.pending);
		state.close_server_admin();
		deliver(&mut state, guild, request, Ok(Outcome::Invites(page())));
		assert!(state.server_admin.invites.is_none());
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(Id(2), Action::Invites(InviteAction::Load))
			.unwrap()
		else {
			panic!()
		};
		let mut role = state
			.permissions
			.guilds
			.get(&guild)
			.unwrap()
			.roles
			.as_ref()
			.unwrap()[0]
			.clone();
		role.bits = p::MANAGE_ROLES;
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Permissions(crate::permissions::Event::Role { guild, role }),
		});
		assert!(!state.can_open_invite_settings(guild));
		deliver(&mut state, guild, request, Ok(Outcome::Invites(page())));
		assert!(state.server_admin.invites.is_none());
	}
	#[test]
	fn invites_revoke_and_pause_require_confirmed_results_and_explicit_reload_after_ambiguity() {
		let mut state = state();
		let revoke = Action::Invites(InviteAction::Revoke {
			code: "synthetic_code".into(),
		});
		let Command::ServerAdmin { guild, request, .. } =
			state.request_server_admin(Id(2), revoke.clone()).unwrap()
		else {
			panic!()
		};
		deliver(&mut state, guild, request, Err(Failure::Ambiguous));
		assert!(state.server_admin.needs_refresh);
		assert!(state.request_server_admin(guild, revoke.clone()).is_none());
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(guild, Action::Invites(InviteAction::Load))
			.unwrap()
		else {
			panic!()
		};
		deliver(&mut state, guild, request, Ok(Outcome::Invites(page())));
		assert!(!state.server_admin.needs_refresh);
		let Command::ServerAdmin { request, .. } =
			state.request_server_admin(guild, revoke).unwrap()
		else {
			panic!()
		};
		let mut empty = page();
		empty.items.clear();
		deliver(&mut state, guild, request, Ok(Outcome::Invites(empty)));
		assert!(
			state
				.server_admin
				.invites
				.as_ref()
				.unwrap()
				.items
				.is_empty()
		);
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(
				guild,
				Action::Invites(InviteAction::SetPaused { paused: true }),
			)
			.unwrap()
		else {
			panic!()
		};
		deliver(&mut state, guild, request, Ok(Outcome::Invites(page())));
		assert!(state.server_admin.needs_refresh);
		assert!(!state.server_admin.invites.as_ref().unwrap().paused());
	}
}

#[cfg(test)]
mod sticker_tests {
	use super::*;

	fn user(id: u64, name: &str) -> model::User {
		model::User {
			primary_guild: None,
			id: Id(id),
			name: name.into(),
			avatar: None,
			discriminator: 0,
			kind: Default::default(),
			webhook: false,
		}
	}
	fn row(id: u64, name: &str, uploader: u64) -> model::server_admin::Sticker {
		model::server_admin::Sticker {
			sticker: model::Sticker {
				id: Id(id),
				name: name.into(),
				description: "A friendly wave".into(),
				tags: "wave".into(),
				format_type: 1,
				guild_id: Some(Id(2)),
				pack_id: None,
				available: true,
			},
			uploader: Some(user(uploader, "Uploader")),
		}
	}
	fn state(bits: u128) -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(user(1, "Synthetic")),
			guilds: vec![model::Guild {
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				stickers: None,
				premium_tier: 0,
			}],
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			p::Guild {
				id: Id(2),
				owner: Some(Id(99)),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
				roles: Some(vec![p::Role {
					id: Id(2),
					name: "@everyone".into(),
					bits,
					color: 0,
					position: 0,
					hoist: false,
				}]),
			},
		);
		state.server_admin.guild = Some(Id(2));
		state.server_admin.stickers = Some(model::server_admin::Stickers {
			items: vec![row(4, "Wave", 1), row(5, "Other", 7)],
			limit: Some(5),
		});
		state
	}

	#[test]
	fn sticker_permissions_follow_creator_and_manager_rules() {
		let mut state = state(p::CREATE_GUILD_EXPRESSIONS);
		assert!(state.can_open_sticker_settings(Id(2)));
		assert!(state.can_create_guild_sticker(Id(2)));
		assert!(state.can_edit_guild_sticker(Id(2), Id(4)));
		assert!(!state.can_edit_guild_sticker(Id(2), Id(5)));

		state
			.permissions
			.guilds
			.get_mut(&Id(2))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits = p::MANAGE_GUILD_EXPRESSIONS;
		state.permissions.clear_cache();
		assert!(!state.can_create_guild_sticker(Id(2)));
		assert!(state.can_edit_guild_sticker(Id(2), Id(5)));
	}

	#[test]
	fn sticker_create_drops_retained_file_and_reconciles_guild_catalog() {
		let mut state = state(p::CREATE_GUILD_EXPRESSIONS);
		let action = Action::CreateSticker {
			name: "New Sticker".into(),
			description: "A friendly wave".into(),
			tags: "wave".into(),
			filename: "wave.png".into(),
			content_type: "image/png".into(),
			file: vec![1, 2, 3],
		};
		let Command::ServerAdmin {
			request, action, ..
		} = state.request_server_admin(Id(2), action).unwrap()
		else {
			panic!()
		};
		assert!(matches!(*action, Action::CreateSticker { ref file, .. } if file == &[1, 2, 3]));
		assert!(
			matches!(state.server_admin.action, Some(Action::CreateSticker { ref file, .. }) if file.is_empty())
		);

		let page = model::server_admin::Stickers {
			items: vec![row(6, "New Sticker", 1)],
			limit: Some(5),
		};
		state
			.apply_server_admin(Event {
				guild: Id(2),
				request,
				result: Ok(Outcome::Stickers(page)),
			})
			.unwrap();
		assert!(
			state.guild(Id(2)).unwrap().stickers.is_some(),
			"status={} admin={:?}",
			state.status,
			state.server_admin.error
		);
		assert_eq!(
			state.guild(Id(2)).unwrap().stickers.as_ref().unwrap()[0].id,
			Id(6)
		);
		assert_eq!(
			state.server_admin.stickers.as_ref().unwrap().items[0]
				.sticker
				.id,
			Id(6)
		);
	}
}
