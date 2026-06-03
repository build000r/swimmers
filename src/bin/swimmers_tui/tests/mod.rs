use super::*;

mod api_client;
mod background;
mod events;
mod glance;
mod help_footer;
mod key_events;
mod layout;
mod mermaid_basic;
mod mermaid_er;
mod mermaid_interaction;
mod mermaid_metamorphic;
mod mermaid_plan_tabs;
mod mermaid_rendering;
mod mermaid_semantic;
mod paste_publication;
mod picker;
mod plans;
mod refresh;
mod session_input;
mod skill_panel;
mod terminal_lifecycle;
mod thought_config_render;
mod thought_history;
mod thought_panel;

use key_events::thought_config_test_editor;
use mermaid_plan_tabs::{open_mermaid_on_plan_tab, open_mermaid_with_plan_tabs};

use std::cell::Cell as TestCell;
use std::collections::VecDeque;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use chrono::Utc;
use proptest::prelude::*;
use swimmers::api::remote_sessions;
use swimmers::openrouter_models::default_openrouter_candidates;
use swimmers::types::{
    AttentionGroupLayout, CreateSessionsBatchResult, GhosttyOpenMode, RepoActionStatus,
    SessionBatchMembership, SessionGroupInputResult, StateEvidence, ThoughtSource, ThoughtState,
    TransportHealth,
};
use tempfile::tempdir;

include!("api_mock.rs");
include!("support.rs");
include!("render_support.rs");
include!("mermaid_support.rs");
include!("fixtures.rs");
