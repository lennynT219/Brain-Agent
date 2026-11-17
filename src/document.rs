use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use uuid::Uuid;

pub const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommonMetadata {
    pub brain_version: u32,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub brain_id: Uuid,
    pub title: String,
}

/// Owned metadata only; not a preservation codec for existing Markdown files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "brain_type", rename_all = "lowercase")]
pub enum DocumentMetadata {
    Note(CommonMetadata),
    Project(CommonMetadata),
    Memory {
        #[serde(flatten)]
        common: CommonMetadata,
        memory_revision: u64,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_uuid",
            skip_serializing_if = "Option::is_none"
        )]
        brain_project: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_link: Option<String>,
    },
    Task {
        #[serde(flatten)]
        common: CommonMetadata,
        #[serde(deserialize_with = "deserialize_uuid")]
        brain_project: Uuid,
        project_link: String,
        status: TaskStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "Pendiente",
            Self::InProgress => "En curso",
            Self::Done => "Terminada",
            Self::Cancelled => "Cancelada",
        })
    }
}

fn deserialize_uuid<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let value = String::deserialize(deserializer)?;
    Uuid::parse_str(&value).map_err(|_| serde::de::Error::custom("UUID inválido"))
}

fn deserialize_optional_uuid<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Uuid>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| Uuid::parse_str(&value).map_err(|_| serde::de::Error::custom("UUID inválido")))
        .transpose()
}

/// Scalar syntax only; this does not resolve a note or grant access to it.
fn qualified_wikilink(link: &str) -> bool {
    let Some(inner) = link.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) else {
        return false;
    };
    let Some((target, label)) = inner.split_once('|') else {
        return false;
    };
    !label.trim().is_empty()
        && !label.contains(['[', ']', '|', '\n', '\r'])
        && target.contains('/')
        && !target.contains(['[', ']', '|', '\\', ':', '#', '\n', '\r'])
        && target
            .split('/')
            .all(|part| !part.trim().is_empty() && part != "." && part != "..")
}

impl DocumentMetadata {
    pub fn common(&self) -> &CommonMetadata {
        match self {
            Self::Note(common)
            | Self::Project(common)
            | Self::Memory { common, .. }
            | Self::Task { common, .. } => common,
        }
    }

    /// Checks scalars only, not path safety or the referenced project's identity.
    pub fn validate(&self) -> Result<(), &'static str> {
        let common = self.common();
        if common.brain_version != DOCUMENT_VERSION {
            return Err("Versión de documento no compatible");
        }
        if common.title.trim().is_empty() {
            return Err("El título no puede estar vacío");
        }
        if let Self::Memory {
            memory_revision,
            brain_project,
            project_link,
            ..
        } = self
        {
            if common.brain_id.is_nil() || *memory_revision == 0 {
                return Err("Memoria requiere identidad no nula y revisión positiva");
            }
            match (brain_project, project_link) {
                (None, None) => {}
                (Some(id), Some(link)) if !id.is_nil() && qualified_wikilink(link) => {}
                _ => {
                    return Err(
                        "Proyecto opcional requiere UUID y enlace cualificado con etiqueta",
                    );
                }
            }
        }
        if let Self::Task { project_link, .. } = self {
            let target = project_link
                .strip_prefix("[[")
                .and_then(|link| link.strip_suffix("]]"));
            if !target
                .is_some_and(|target| !target.trim().is_empty() && !target.contains(['[', ']']))
            {
                return Err("El enlace del proyecto debe tener formato [[destino]]");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    // TOML is only an in-memory Serde test harness, not the notes storage format.
    fn fixture(kind: &str) -> String {
        let mut input = format!(
            "brain_type = {kind:?}\nbrain_version = 1\nbrain_id = {ID:?}\ntitle = \"Título\"\n"
        );
        if kind == "task" {
            input.push_str(&format!(
                "brain_project = {ID:?}\nproject_link = \"[[Proyecto]]\"\nstatus = \"pending\"\n"
            ));
        }
        input
    }

    #[test]
    fn unsupported_version_is_rejected() {
        for kind in ["note", "project", "task"] {
            for version in [0, 2, u32::MAX] {
                let input = fixture(kind)
                    .replace("brain_version = 1", &format!("brain_version = {version}"));
                let metadata: DocumentMetadata = toml::from_str(&input).unwrap();
                assert_eq!(
                    metadata.validate(),
                    Err("Versión de documento no compatible")
                );
            }
        }
        let mut metadata: DocumentMetadata = toml::from_str(&fixture("task")).unwrap();
        if let DocumentMetadata::Task {
            common,
            project_link,
            ..
        } = &mut metadata
        {
            common.brain_version = 2;
            common.title.clear();
            project_link.clear();
        }
        assert_eq!(
            metadata.validate(),
            Err("Versión de documento no compatible")
        );
    }

    #[test]
    fn tags_and_known_fields_roundtrip() {
        for kind in ["note", "project", "task"] {
            let metadata: DocumentMetadata = toml::from_str(&fixture(kind)).unwrap();
            assert!(matches!(
                (&metadata, kind),
                (DocumentMetadata::Note(_), "note")
                    | (DocumentMetadata::Project(_), "project")
                    | (DocumentMetadata::Task { .. }, "task")
            ));
            assert_eq!(metadata.common().brain_id, Uuid::parse_str(ID).unwrap());
            assert!(metadata.validate().is_ok());
            let encoded = toml::to_string(&metadata).unwrap();
            let wire: toml::Table = toml::from_str(&encoded).unwrap();
            assert_eq!(wire["brain_type"].as_str(), Some(kind));
            assert_eq!(wire["title"].as_str(), Some("Título"));
            assert_eq!(wire["brain_id"].as_str(), Some(ID));
            assert!(!wire.contains_key("common"));
            assert_eq!(
                toml::from_str::<DocumentMetadata>(&encoded).unwrap(),
                metadata
            );
            let extra = format!(
                "{}additional = \"retained by the future reader\"\n",
                fixture(kind)
            );
            assert_eq!(
                toml::from_str::<DocumentMetadata>(&extra).unwrap(),
                metadata
            );
        }
    }

    #[test]
    fn required_fields_are_not_defaulted() {
        for kind in ["note", "project", "task"] {
            for field in [
                "brain_type",
                "brain_version",
                "brain_id",
                "title",
                "brain_project",
                "project_link",
                "status",
            ] {
                if kind != "task" && ["brain_project", "project_link", "status"].contains(&field) {
                    continue;
                }
                let input = fixture(kind)
                    .lines()
                    .filter(|line| !line.starts_with(&format!("{field} =")))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    toml::from_str::<DocumentMetadata>(&input).is_err(),
                    "{kind}: {field}"
                );
            }
        }
    }

    #[test]
    fn invalid_values_and_non_string_uuids_are_rejected() {
        for (field, values) in [
            ("brain_type", vec!["\"unknown\"", "1"]),
            ("status", vec!["\"unknown\"", "\"InProgress\"", "1"]),
            (
                "brain_id",
                vec![
                    "\"invalid\"",
                    "1",
                    "[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                ],
            ),
            (
                "brain_project",
                vec![
                    "\"invalid\"",
                    "1",
                    "[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                ],
            ),
        ] {
            for value in values {
                let mut wire: toml::Table = toml::from_str(&fixture("task")).unwrap();
                let replacement: toml::Table = toml::from_str(&format!("value = {value}")).unwrap();
                wire.insert(field.into(), replacement["value"].clone());
                assert!(
                    wire.try_into::<DocumentMetadata>().is_err(),
                    "{field}: {value}"
                );
            }
        }
    }

    #[test]
    fn valid_uuid_formats_and_versions_are_accepted() {
        for id in [
            ID.to_uppercase(),
            ID.replace('-', ""),
            format!("urn:uuid:{ID}"),
            format!("{{{ID}}}"),
            Uuid::nil().to_string(),
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8".into(),
        ] {
            let input = fixture("task").replace(ID, &id);
            let metadata: DocumentMetadata = toml::from_str(&input).unwrap();
            assert!(metadata.validate().is_ok(), "{id}");
        }
    }

    #[test]
    fn titles_and_basic_wikilinks_are_validated_without_mutation() {
        for title in ["", " ", "\t\n", "  Título  "] {
            let mut metadata: DocumentMetadata = toml::from_str(&fixture("task")).unwrap();
            if let DocumentMetadata::Task { common, .. } = &mut metadata {
                common.title = title.into();
            }
            assert_eq!(metadata.validate().is_ok(), !title.trim().is_empty());
            assert_eq!(metadata.common().title, title);
        }
        for link in [
            "",
            "Proyecto",
            "[Proyecto]",
            "[[Proyecto]",
            "[Proyecto]]",
            "[[]]",
            "[[  ]]",
            "[[[Proyecto]]]",
            "[[Uno]][[Dos]]",
            "[[Proyecto]]extra",
        ] {
            let mut metadata: DocumentMetadata = toml::from_str(&fixture("task")).unwrap();
            if let DocumentMetadata::Task { project_link, .. } = &mut metadata {
                *project_link = link.into();
            }
            assert!(metadata.validate().is_err(), "{link}");
        }
    }

    #[test]
    fn all_task_states_have_machine_values_and_spanish_labels() {
        for (status, machine, label) in [
            (TaskStatus::Pending, "pending", "Pendiente"),
            (TaskStatus::InProgress, "in_progress", "En curso"),
            (TaskStatus::Done, "done", "Terminada"),
            (TaskStatus::Cancelled, "cancelled", "Cancelada"),
        ] {
            let input = fixture("task").replace("pending", machine);
            let metadata: DocumentMetadata = toml::from_str(&input).unwrap();
            if let DocumentMetadata::Task { status: actual, .. } = &metadata {
                assert_eq!(actual, &status);
            }
            let wire: toml::Table = toml::from_str(&toml::to_string(&metadata).unwrap()).unwrap();
            assert_eq!(wire["status"].as_str(), Some(machine));
            assert_eq!(status.to_string(), label);
        }
    }
}
