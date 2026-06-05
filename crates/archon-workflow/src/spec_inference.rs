use crate::spec::{StageKind, StageSpec};

pub(crate) fn fanout_requires_item_kind(stage: &StageSpec) -> bool {
    stage.infers_implementation_fanout()
        && (stage.foreach.as_deref().is_some_and(has_text)
            || stage
                .input
                .get("items")
                .and_then(serde_json::Value::as_array)
                .is_some())
}

pub(crate) fn missing_item_kind_error(stage: &StageSpec) -> Option<String> {
    fanout_requires_item_kind(stage).then(|| {
        format!(
            "fanout stage '{}' appears to implement repository changes; set `item_kind: implementation`",
            stage.id
        )
    })
}

impl StageSpec {
    pub fn effective_item_kind(&self) -> StageKind {
        if self.kind == StageKind::Fanout
            && (self.item_kind == Some(StageKind::Implementation)
                || self.infers_implementation_fanout())
        {
            return StageKind::Implementation;
        }
        self.item_kind.unwrap_or(self.kind)
    }

    pub fn write_capable(&self) -> bool {
        self.kind == StageKind::Implementation
            || (self.kind == StageKind::Fanout
                && self.effective_item_kind() == StageKind::Implementation)
    }

    pub fn infers_implementation_fanout(&self) -> bool {
        if self.kind != StageKind::Fanout || self.item_kind.is_some() {
            return false;
        }
        let id = self.id.to_ascii_lowercase();
        let task = self
            .task
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        id == "implement"
            || id.starts_with("implement_")
            || id.starts_with("implement-")
            || task.contains("implement only")
            || task.contains("implement the")
            || task.contains("write-capable")
            || task.contains("modify repository")
            || task.contains("modify the repository")
    }
}

fn has_text(value: &str) -> bool {
    !value.trim().is_empty()
}
