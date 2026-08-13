use std::path::Path;

use askama::Template;
use lungyam_core::{
    config::Config,
    config_diff::ConfigDiff,
    revision::{FileRevisionStore, RevisionMetadata},
    revision_state::FileRevisionStateStore,
};
use serde::Deserialize;

use crate::security;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct DiffQuery {
    pub from: u64,
    pub to: u64,
}

#[derive(Clone, Debug)]
struct RevisionView {
    revision: u64,
    label: String,
    created_at: String,
    actor: String,
    reason: String,
    status: String,
}

#[derive(Template)]
#[template(path = "revisions.html")]
struct RevisionsTemplate {
    overview_active: bool,
    routes_active: bool,
    available: bool,
    message: String,
    active_revision: String,
    pending_revision: String,
    has_pending: bool,
    writes_enabled: bool,
    csrf_token: String,
    can_diff: bool,
    revisions: Vec<RevisionView>,
}

#[derive(Template)]
#[template(path = "fragments/config-diff.html")]
struct ConfigDiffTemplate {
    valid: bool,
    message: String,
    from_revision: String,
    to_revision: String,
    empty: bool,
    restart_required: bool,
    server_changed: bool,
    admin_changed: bool,
    upstreams_added: String,
    upstreams_removed: String,
    upstreams_changed: String,
    routes_added: String,
    routes_removed: String,
    routes_changed: String,
}

pub(crate) fn render_revisions(config_path: Option<&Path>, config: &Config) -> Result<String, String> {
    let csrf_token = security::csrf_token().expose().to_owned();
    let Some(config_path) = config_path else {
        return RevisionsTemplate {
            overview_active: false,
            routes_active: false,
            available: false,
            message: "Revision storage is unavailable because the admin server was started without a config path."
                .to_owned(),
            active_revision: "Not set".to_owned(),
            pending_revision: "Not set".to_owned(),
            has_pending: false,
            writes_enabled: false,
            csrf_token,
            can_diff: false,
            revisions: Vec::new(),
        }
        .render()
        .map_err(|error| error.to_string());
    };

    let store = FileRevisionStore::beside_config(config_path);
    let state_store = FileRevisionStateStore::new(store.root());
    let state = state_store.load().map_err(|error| error.to_string())?;
    let metadata = store.list().map_err(|error| error.to_string())?;
    let revisions = metadata
        .iter()
        .map(|item| revision_view(item, state.active_revision, state.pending_revision))
        .collect::<Vec<_>>();

    RevisionsTemplate {
        overview_active: false,
        routes_active: false,
        available: true,
        message: if revisions.is_empty() {
            "No persisted revisions yet.".to_owned()
        } else {
            "Immutable snapshots stored beside the active configuration.".to_owned()
        },
        active_revision: revision_label(state.active_revision),
        pending_revision: revision_label(state.pending_revision),
        has_pending: state.pending_revision.is_some(),
        writes_enabled: security::writes_enabled(config),
        csrf_token,
        can_diff: revisions.len() >= 2,
        revisions,
    }
    .render()
    .map_err(|error| error.to_string())
}

pub(crate) fn render_diff(config_path: Option<&Path>, query: DiffQuery) -> Result<String, String> {
    let Some(config_path) = config_path else {
        return render_diff_error("Revision storage is unavailable.");
    };

    let store = FileRevisionStore::beside_config(config_path);
    let from = match store.load(query.from) {
        Ok(revision) => revision,
        Err(error) => return render_diff_error(&error.to_string()),
    };
    let to = match store.load(query.to) {
        Ok(revision) => revision,
        Err(error) => return render_diff_error(&error.to_string()),
    };
    let diff = ConfigDiff::between(&from.config, &to.config);

    ConfigDiffTemplate {
        valid: true,
        message: if diff.is_empty() {
            "The selected revisions contain the same effective configuration.".to_owned()
        } else if diff.restart_required() {
            "This diff contains structural changes that require a process restart after activation."
                .to_owned()
        } else {
            "This route-only diff can be applied to the running proxy without a restart."
                .to_owned()
        },
        from_revision: format!("#{:06}", query.from),
        to_revision: format!("#{:06}", query.to),
        empty: diff.is_empty(),
        restart_required: diff.restart_required(),
        server_changed: diff.server_changed,
        admin_changed: diff.admin_changed,
        upstreams_added: format_list(&diff.upstreams_added),
        upstreams_removed: format_list(&diff.upstreams_removed),
        upstreams_changed: format_list(&diff.upstreams_changed),
        routes_added: format_list(&diff.routes_added),
        routes_removed: format_list(&diff.routes_removed),
        routes_changed: format_list(&diff.routes_changed),
    }
    .render()
    .map_err(|error| error.to_string())
}

fn render_diff_error(message: &str) -> Result<String, String> {
    ConfigDiffTemplate {
        valid: false,
        message: message.to_owned(),
        from_revision: String::new(),
        to_revision: String::new(),
        empty: false,
        restart_required: false,
        server_changed: false,
        admin_changed: false,
        upstreams_added: String::new(),
        upstreams_removed: String::new(),
        upstreams_changed: String::new(),
        routes_added: String::new(),
        routes_removed: String::new(),
        routes_changed: String::new(),
    }
    .render()
    .map_err(|error| error.to_string())
}

fn revision_view(
    metadata: &RevisionMetadata,
    active: Option<u64>,
    pending: Option<u64>,
) -> RevisionView {
    RevisionView {
        revision: metadata.revision,
        label: format!("#{:06}", metadata.revision),
        created_at: format!("{} (Unix)", metadata.created_at_unix_seconds),
        actor: metadata.actor.clone().unwrap_or_else(|| "—".to_owned()),
        reason: metadata.reason.clone().unwrap_or_else(|| "—".to_owned()),
        status: if active == Some(metadata.revision) {
            "Active".to_owned()
        } else if pending == Some(metadata.revision) {
            "Pending".to_owned()
        } else {
            "Stored".to_owned()
        },
    }
}

fn revision_label(revision: Option<u64>) -> String {
    revision.map_or_else(|| "Not set".to_owned(), |value| format!("#{value:06}"))
}

fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_owned()
    } else {
        items.join(", ")
    }
}
