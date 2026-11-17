//! Local Markdown Memory core, not a turn hook or an authority service.
//!
//! The trusted caller supplies authorized paths and operations. Identifiers,
//! stored content and returned paths never grant permission. Reads never create.

pub mod binding;
pub mod history;
pub mod markdown;
pub mod store;
mod syntax;

pub const CURRENT_SECTIONS: [&str; 5] = [
    "Objective and scope",
    "Current decisions",
    "Outcome status",
    "Relevant evidence",
    "Next actions and open questions",
];

/// Fixed, short basenames; UUIDs belong in the directory, not the note name.
pub fn current_link(memory_id: uuid::Uuid) -> String {
    format!("[[05-memorias/{memory_id}/Memoria|Memoria]]")
}

pub fn history_link(memory_id: uuid::Uuid) -> String {
    format!("[[05-memorias/{memory_id}/Historial|Historial]]")
}
