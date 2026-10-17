//! GUI-only Save / dirty-replacement orchestration using existing engine actions.
//! The editor remains the only draft owner. Copies here are immutable submitted
//! snapshots or explicit replacement candidates, never a second editable model.
use super::{Action, Reply, setup_library_panel::Report};
use underglow::{
    setup::SetupDocument,
    setup_library::{LibraryMutation, same_content},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lighting {
    Paused,
    Active,
    Unknown,
}

pub(super) struct View<'a> {
    pub draft: Option<&'a SetupDocument>,
    pub dirty: bool,
    pub setup_revision: u64,
    pub connected: bool,
    pub busy: bool,
    pub lighting: Lighting,
    pub library: Option<&'a Report>,
}

#[derive(Clone, Debug)]
pub(super) enum Destination {
    Saved(String),
    NewDraft(SetupDocument),
    ReloadSaved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prompt {
    DirtyReplacement,
    PauseAndSave,
    PauseAndOpen,
}

#[derive(Clone, Copy)]
pub(super) enum DirtyChoice {
    Save,
    Discard,
    Cancel,
}

/// Apply the saved baseline/revision even when `expected_draft` no longer
/// matches. Only replace/clean the editor draft if that exact snapshot matches;
/// a newer draft must remain dirty. This also handles an acknowledged switch
/// after edits arrived in flight, without lying about what the engine installed.
#[derive(Debug)]
pub(super) struct Accepted {
    pub expected_draft: Option<SetupDocument>,
    pub document: SetupDocument,
}

#[derive(Default)]
pub(super) struct Update {
    pub action: Option<Action>,
    pub accepted: Option<Accepted>,
    /// Explicit local replacement; never persistence, switching or activation.
    pub new_draft: Option<SetupDocument>,
    pub notice: Option<String>,
}

#[derive(Clone)]
struct Save {
    submitted: SetupDocument,
    revision: u64,
    library_revision: Option<u64>,
    after: Option<Destination>,
}

#[derive(Clone)]
struct Open {
    id: String,
    revision: u64,
    library_revision: u64,
    expected_draft: Option<SetupDocument>,
}

#[derive(Clone)]
enum Operation {
    Save(Save),
    Open(Open),
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Dirty(Destination),
    ConfirmPause(Operation),
    Pausing(Operation),
    Saving(Save),
    Refreshing {
        save: Save,
        saved: SetupDocument,
    },
    Opening(Open),
    Reloading(Option<SetupDocument>),
}

#[derive(Default)]
pub(super) struct Workflow {
    state: State,
}

impl Workflow {
    pub fn owns(&self, action: &Action) -> bool {
        self.matches(action)
    }

    pub fn abort(&mut self) {
        self.state = State::Idle;
    }

    pub fn prompt(&self) -> Option<Prompt> {
        match self.state {
            State::Dirty(_) => Some(Prompt::DirtyReplacement),
            State::ConfirmPause(Operation::Save(_)) => Some(Prompt::PauseAndSave),
            State::ConfirmPause(Operation::Open(_)) => Some(Prompt::PauseAndOpen),
            _ => None,
        }
    }

    pub fn pending(&self) -> bool {
        matches!(
            self.state,
            State::Pausing(_)
                | State::Saving(_)
                | State::Refreshing { .. }
                | State::Opening(_)
                | State::Reloading(_)
        )
    }

    /// Cancelling a prompt has no transport, document or lighting effect.
    /// An already-dispatched operation cannot be cancelled as if it never ran.
    pub fn cancel(&mut self) {
        if self.prompt().is_some() {
            self.state = State::Idle;
        }
    }

    pub fn request_save(&mut self, view: &View<'_>) -> Result<Update, String> {
        self.available(view)?;
        self.start_save(view, None)
    }

    pub fn request_destination(
        &mut self,
        destination: Destination,
        view: &View<'_>,
    ) -> Result<Update, String> {
        self.available(view)?;
        validate_destination(&destination, view)?;
        if view.dirty {
            self.state = State::Dirty(destination);
            Ok(Update::default())
        } else {
            self.prepare_destination(destination, view)
        }
    }

    pub fn resolve_dirty(
        &mut self,
        choice: DirtyChoice,
        view: &View<'_>,
    ) -> Result<Update, String> {
        let State::Dirty(destination) = &self.state else {
            return Err("No setup replacement is awaiting a choice".into());
        };
        if view.busy {
            return Err("Wait for the current request; draft retained".into());
        }
        let destination = destination.clone();
        self.state = State::Idle;
        match choice {
            DirtyChoice::Cancel => Ok(Update::default()),
            DirtyChoice::Save => self.start_save(view, Some(destination)),
            // Do not discard before a remote switch succeeds. Its acknowledgement
            // carries the old draft guard so failures retain even this draft.
            DirtyChoice::Discard => self.prepare_destination(destination, view),
        }
    }

    pub fn confirm_pause(&mut self, view: &View<'_>) -> Result<Update, String> {
        let State::ConfirmPause(operation) = &self.state else {
            return Err("No pause is awaiting confirmation".into());
        };
        ready(view)?;
        let operation = operation.clone();
        ensure_current(&operation, view)?;
        self.state = State::Pausing(operation);
        Ok(Update {
            action: Some(Action::Control(vec!["pause".into()])),
            ..Default::default()
        })
    }

    /// Only the completion of the exact dispatched action can advance a flow.
    /// In particular a successful status poll can NEVER acknowledge Pause.
    /// The host still updates actual connection/lighting status independently.
    pub fn complete(
        &mut self,
        action: &Action,
        reply: Result<&Reply, &str>,
        view: &View<'_>,
    ) -> Result<Update, String> {
        if !self.matches(action) {
            return Ok(Update::default());
        }
        let state = std::mem::take(&mut self.state);
        let reply = reply.map_err(|error| format!("{error}; draft retained"))?;
        if !reply.ok {
            return Err(reply
                .error
                .clone()
                .unwrap_or_else(|| "Request failed; draft retained".into()));
        }
        match state {
            State::Pausing(operation) => {
                if !reply_is_paused(reply) {
                    return Err("Pause was not acknowledged as paused; draft retained".into());
                }
                ensure_current(&operation, view)?;
                self.dispatch_operation(operation)
            }
            State::Saving(save) => self.saved(save, reply, view),
            State::Refreshing { save, saved } => {
                if view.draft != Some(&saved) {
                    return Err("Newer draft retained; opening another setup cancelled".into());
                }
                if !reply_is_paused(reply) {
                    return Err(
                        "Saved; refresh paused lighting status before opening a setup".into(),
                    );
                }
                let report = saved_report(reply, &save, &saved)?;
                self.after_save(save.after, &saved, report)
            }
            State::Opening(open) => {
                let saved = acknowledged_document(reply, open.revision)?;
                if saved.id != open.id {
                    return Err("Unexpected setup identity; draft retained".into());
                }
                Ok(Update {
                    accepted: Some(Accepted {
                        expected_draft: open.expected_draft,
                        document: saved,
                    }),
                    notice: Some(
                        "Opened saved setup. Enable lighting separately when ready.".into(),
                    ),
                    ..Default::default()
                })
            }
            State::Reloading(expected_draft) => {
                let envelope = reply
                    .setup
                    .as_ref()
                    .ok_or("Missing saved setup; draft retained")?;
                let document = envelope
                    .document
                    .as_ref()
                    .ok_or("No canonical saved setup; draft retained")?;
                if document.revision != envelope.revision || document.validate().is_err() {
                    return Err("Invalid saved setup acknowledgement; draft retained".into());
                }
                Ok(Update {
                    accepted: Some(Accepted {
                        expected_draft,
                        document: document.clone(),
                    }),
                    notice: Some("Reloaded saved version. Lighting state was not changed.".into()),
                    ..Default::default()
                })
            }
            _ => unreachable!("matches excludes prompts and idle state"),
        }
    }

    /// Save a copy uses the CURRENT draft, not its applied baseline, and Store
    /// stays inactive. The host must not feed a Store reply into editor loading.
    pub fn save_copy(
        &self,
        view: &View<'_>,
        new_id: String,
        name: String,
    ) -> Result<Action, String> {
        self.available(view)?;
        ready(view)?;
        let report = library(view)?;
        let mut document = view.draft.cloned().ok_or("No setup draft to copy")?;
        if document.id == new_id
            || report.current_id.as_ref() == Some(&new_id)
            || report.setups.iter().any(|entry| entry.id == new_id)
        {
            return Err("Save a copy requires a new setup identity".into());
        }
        document.id = new_id;
        document.name = name;
        document.revision = 0;
        document.validate().map_err(|error| error.to_string())?;
        Ok(Action::LibraryUpdate {
            revision: report.revision,
            mutation: LibraryMutation::Store { document },
        })
    }

    fn available(&self, view: &View<'_>) -> Result<(), String> {
        if !matches!(self.state, State::Idle) || view.busy {
            Err("Finish the current request or choice; draft retained".into())
        } else {
            Ok(())
        }
    }

    fn start_save(
        &mut self,
        view: &View<'_>,
        after: Option<Destination>,
    ) -> Result<Update, String> {
        ready(view)?;
        let document = view.draft.cloned().ok_or("No setup draft to save")?;
        document.validate().map_err(|error| error.to_string())?;
        view.setup_revision
            .checked_add(1)
            .ok_or("Setup revision exhausted")?;
        let operation = Operation::Save(Save {
            submitted: document,
            revision: view.setup_revision,
            library_revision: view.library.map(|report| report.revision),
            after,
        });
        self.prepare_operation(operation, view.lighting)
    }

    fn prepare_destination(
        &mut self,
        destination: Destination,
        view: &View<'_>,
    ) -> Result<Update, String> {
        validate_destination(&destination, view)?;
        match destination {
            Destination::ReloadSaved => {
                ready(view)?;
                self.state = State::Reloading(view.draft.cloned());
                Ok(Update {
                    action: Some(Action::SetupShow),
                    ..Default::default()
                })
            }
            Destination::NewDraft(document) => Ok(Update {
                new_draft: Some(document),
                ..Default::default()
            }),
            Destination::Saved(id) => {
                ready(view)?;
                let report = library(view)?;
                self.prepare_operation(
                    Operation::Open(Open {
                        id,
                        revision: view.setup_revision,
                        library_revision: report.revision,
                        expected_draft: view.draft.cloned(),
                    }),
                    view.lighting,
                )
            }
        }
    }

    fn prepare_operation(
        &mut self,
        operation: Operation,
        lighting: Lighting,
    ) -> Result<Update, String> {
        match lighting {
            Lighting::Paused => self.dispatch_operation(operation),
            Lighting::Active => {
                self.state = State::ConfirmPause(operation);
                Ok(Update::default())
            }
            Lighting::Unknown => Err("Refresh engine status before saving or opening a setup; local edits remain available".into()),
        }
    }

    fn dispatch_operation(&mut self, operation: Operation) -> Result<Update, String> {
        let action = match &operation {
            Operation::Save(save) => Action::SetupReplace {
                revision: save.revision,
                document: save.submitted.clone(),
            },
            Operation::Open(open) => Action::LibraryUpdate {
                revision: open.library_revision,
                mutation: LibraryMutation::Switch {
                    id: open.id.clone(),
                    expected_setup_revision: open.revision,
                },
            },
        };
        self.state = match operation {
            Operation::Save(save) => State::Saving(save),
            Operation::Open(open) => State::Opening(open),
        };
        Ok(Update {
            action: Some(action),
            ..Default::default()
        })
    }

    fn matches(&self, action: &Action) -> bool {
        match (&self.state, action) {
            (State::Pausing(_), Action::Control(args)) => args.len() == 1 && args[0] == "pause",
            (State::Saving(save), Action::SetupReplace { revision, document }) => {
                save.revision == *revision && save.submitted == *document
            }
            (State::Refreshing { .. }, Action::LibraryList) => true,
            (State::Reloading(_), Action::SetupShow) => true,
            (
                State::Opening(open),
                Action::LibraryUpdate {
                    revision,
                    mutation:
                        LibraryMutation::Switch {
                            id,
                            expected_setup_revision,
                        },
                },
            ) => {
                open.library_revision == *revision
                    && open.id == *id
                    && open.revision == *expected_setup_revision
            }
            _ => false,
        }
    }

    fn saved(&mut self, save: Save, reply: &Reply, view: &View<'_>) -> Result<Update, String> {
        let saved = acknowledged_document(reply, save.revision)?;
        if !same_content(&save.submitted, &saved) {
            return Err("Saved content did not match the submitted setup; draft retained".into());
        }
        let mut update = Update {
            accepted: Some(Accepted {
                expected_draft: Some(save.submitted.clone()),
                document: saved.clone(),
            }),
            notice: Some("Saved. Enable lighting separately when ready.".into()),
            ..Default::default()
        };
        if view.draft != Some(&save.submitted) {
            update.notice = Some(
                "Earlier version saved; newer draft retained. Any pending open was cancelled."
                    .into(),
            );
        } else if let Some(destination) = &save.after {
            if matches!(destination, Destination::ReloadSaved) {
                return Ok(update); // This exact draft is now the saved version.
            }
            if let Destination::NewDraft(document) = destination {
                update.new_draft = Some(document.clone());
            } else if !reply_is_paused(reply) {
                update.notice = Some(
                    "Saved; refresh paused lighting status before opening another setup".into(),
                );
            } else if reply.library.is_none() {
                self.state = State::Refreshing { save, saved };
                update.action = Some(Action::LibraryList);
            } else {
                match saved_report(reply, &save, &saved) {
                    Ok(report) => match self.after_save(save.after, &saved, report) {
                        Ok(next) => update.action = next.action,
                        Err(error) => update.notice = Some(error),
                    },
                    Err(error) => update.notice = Some(error),
                }
            }
        }
        Ok(update)
    }

    fn after_save(
        &mut self,
        destination: Option<Destination>,
        saved: &SetupDocument,
        report: &Report,
    ) -> Result<Update, String> {
        let Some(Destination::Saved(id)) = destination else {
            return Err("No saved setup is awaiting opening".into());
        };
        if !report.setups.iter().any(|entry| entry.id == id) {
            return Err("Saved; destination no longer exists. Current draft retained".into());
        }
        self.dispatch_operation(Operation::Open(Open {
            id,
            revision: saved.revision,
            library_revision: report.revision,
            expected_draft: Some(saved.clone()),
        }))
    }
}

fn ready(view: &View<'_>) -> Result<(), String> {
    if !view.connected {
        Err("Engine offline. Local edits and testing remain available; draft retained".into())
    } else if view.busy {
        Err("Wait for the current request; draft retained".into())
    } else {
        Ok(())
    }
}

fn library<'a>(view: &'a View<'_>) -> Result<&'a Report, String> {
    view.library
        .filter(|report| report.schema_version == 1)
        .ok_or_else(|| "Refresh saved setups before this action; draft retained".into())
}

fn validate_destination(destination: &Destination, view: &View<'_>) -> Result<(), String> {
    match destination {
        Destination::ReloadSaved => ready(view)?,
        Destination::Saved(id) => {
            if !library(view)?.setups.iter().any(|entry| &entry.id == id) {
                return Err("Saved setup no longer exists; refresh saved setups".into());
            }
        }
        Destination::NewDraft(document) => {
            document.validate().map_err(|error| error.to_string())?;
            if document.revision != 0
                || view.draft.is_some_and(|draft| draft.id == document.id)
                || view
                    .library
                    .is_some_and(|report| report.setups.iter().any(|entry| entry.id == document.id))
            {
                return Err("A new draft requires a new setup identity and revision zero".into());
            }
        }
    }
    Ok(())
}

fn ensure_current(operation: &Operation, view: &View<'_>) -> Result<(), String> {
    let (revision, expected) = match operation {
        Operation::Save(save) => (save.revision, Some(&save.submitted)),
        Operation::Open(open) => (open.revision, open.expected_draft.as_ref()),
    };
    if revision != view.setup_revision || view.draft != expected {
        return Err("Draft changed while awaiting Pause; request cancelled, draft retained".into());
    }
    Ok(())
}

fn reply_is_paused(reply: &Reply) -> bool {
    reply.status.as_ref().is_some_and(|status| {
        status.schema_version == 1 && status.state == "paused" && !status.enabled
    })
}

fn acknowledged_document(reply: &Reply, previous_revision: u64) -> Result<SetupDocument, String> {
    let envelope = reply
        .setup
        .as_ref()
        .ok_or("Missing setup acknowledgement; draft retained")?;
    let document = envelope
        .document
        .as_ref()
        .ok_or("Missing saved document; draft retained")?;
    if previous_revision.checked_add(1) != Some(envelope.revision)
        || document.revision != envelope.revision
    {
        return Err("Unexpected setup revision; draft retained. Refresh before retrying".into());
    }
    document.validate().map_err(|error| error.to_string())?;
    Ok(document.clone())
}

fn saved_report<'a>(
    reply: &'a Reply,
    save: &Save,
    saved: &SetupDocument,
) -> Result<&'a Report, String> {
    let report = reply
        .library
        .as_ref()
        .ok_or("Saved; library acknowledgement missing")?;
    if report.schema_version != 1
        || report.current_id.as_ref() != Some(&saved.id)
        || save
            .library_revision
            .is_some_and(|revision| report.revision <= revision)
        || !report.setups.iter().any(|entry| {
            entry.id == saved.id && entry.name == saved.name && entry.layers == saved.layers.len()
        })
    {
        return Err("Saved; library acknowledgement did not match. Refresh saved setups before opening another setup".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::super::{SetupEnvelope, Status};
    use super::*;
    use underglow::setup_library::{SetupSummary, Starter, starter};

    fn doc(id: &str, revision: u64) -> SetupDocument {
        let mut document = starter(Starter::MatrixPi, id.into(), id.into()).unwrap();
        document.revision = revision;
        document
    }

    fn summary(document: &SetupDocument) -> SetupSummary {
        SetupSummary {
            id: document.id.clone(),
            name: document.name.clone(),
            layers: document.layers.len(),
        }
    }

    fn report(document: &SetupDocument, revision: u64) -> Report {
        Report {
            schema_version: 1,
            revision,
            current_id: Some(document.id.clone()),
            setups: vec![summary(document), summary(&doc("other", 0))],
        }
    }

    fn view<'a>(draft: &'a SetupDocument, library: &'a Report) -> View<'a> {
        View {
            draft: Some(draft),
            dirty: true,
            setup_revision: draft.revision,
            connected: true,
            busy: false,
            lighting: Lighting::Paused,
            library: Some(library),
        }
    }

    fn paused() -> Status {
        Status {
            schema_version: 1,
            state: "paused".into(),
            enabled: false,
            mode: "matrix".into(),
            brightness: 255,
            palette: "ocean".into(),
            fps: 30,
            speed: None,
            ripple_base_color: None,
            ripple_color: None,
            last_error: None,
            retry_attempt: 0,
            migration_warning: None,
        }
    }

    fn reply(saved: Option<&SetupDocument>, library_revision: Option<u64>) -> Reply {
        Reply {
            ok: true,
            error: None,
            status: Some(paused()),
            diagnostics: None,
            setup: saved.map(|document| SetupEnvelope {
                revision: document.revision,
                document: Some(document.clone()),
            }),
            library: library_revision.map(|revision| report(saved.unwrap(), revision)),
        }
    }

    fn saved(document: &SetupDocument) -> SetupDocument {
        let mut saved = document.clone();
        saved.revision += 1;
        saved
    }

    fn request_save_first(flow: &mut Workflow, view: &View<'_>) -> Action {
        assert!(
            flow.request_destination(Destination::Saved("other".into()), view)
                .unwrap()
                .action
                .is_none()
        );
        assert_eq!(flow.prompt(), Some(Prompt::DirtyReplacement));
        flow.resolve_dirty(DirtyChoice::Save, view)
            .unwrap()
            .action
            .unwrap()
    }

    fn assert_switch(action: &Action, library_revision: u64, setup_revision: u64) {
        assert!(matches!(action, Action::LibraryUpdate {
            revision,
            mutation: LibraryMutation::Switch { id, expected_setup_revision },
        } if *revision == library_revision && *expected_setup_revision == setup_revision && id == "other"));
    }

    #[test]
    fn active_save_confirms_pause_and_only_explicit_pause_completion_advances() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        view.lighting = Lighting::Active;
        let mut flow = Workflow::default();
        assert!(flow.request_save(&view).unwrap().action.is_none());
        assert_eq!(flow.prompt(), Some(Prompt::PauseAndSave));
        flow.cancel();
        assert_eq!(flow.prompt(), None);
        assert!(!flow.pending());
        flow.request_save(&view).unwrap();
        let pause = flow.confirm_pause(&view).unwrap().action.unwrap();
        assert!(
            matches!(&pause, Action::Control(args) if args == &[std::ffi::OsString::from("pause")])
        );
        let poll = Action::Control(vec!["status".into()]);
        assert!(
            flow.complete(&poll, Ok(&reply(None, None)), &view)
                .unwrap()
                .action
                .is_none()
        );
        assert!(flow.pending());
        // Cancel cannot pretend a dispatched pause never ran.
        flow.cancel();
        assert!(flow.pending());
        let next = flow
            .complete(&pause, Ok(&reply(None, None)), &view)
            .unwrap();
        assert!(
            matches!(next.action, Some(Action::SetupReplace { revision: 4, document: submitted }) if submitted == document)
        );
    }

    #[test]
    fn malformed_or_unpaused_pause_acknowledgements_abort_without_save() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        view.lighting = Lighting::Active;
        for invalid in 0..5 {
            let mut flow = Workflow::default();
            flow.request_save(&view).unwrap();
            let pause = flow.confirm_pause(&view).unwrap().action.unwrap();
            let mut response = reply(None, None);
            match invalid {
                0 => response.status = None,
                1 => response.status.as_mut().unwrap().enabled = true,
                2 => response.status.as_mut().unwrap().state = "active".into(),
                3 => response.status.as_mut().unwrap().schema_version = 2,
                _ => response.ok = false,
            }
            assert!(flow.complete(&pause, Ok(&response), &view).is_err());
            assert!(!flow.pending());
            assert!(
                flow.complete(&pause, Ok(&reply(None, None)), &view)
                    .unwrap()
                    .action
                    .is_none()
            );
        }
    }

    #[test]
    fn pause_then_failed_save_never_resumes_or_continues() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        view.lighting = Lighting::Active;
        for transport_error in [false, true] {
            let mut flow = Workflow::default();
            flow.request_destination(Destination::Saved("other".into()), &view)
                .unwrap();
            flow.resolve_dirty(DirtyChoice::Save, &view).unwrap();
            let pause = flow.confirm_pause(&view).unwrap().action.unwrap();
            let action = flow
                .complete(&pause, Ok(&reply(None, None)), &view)
                .unwrap()
                .action
                .unwrap();
            let mut failure = reply(None, None);
            failure.ok = false;
            failure.error = Some("Setup revision conflict".into());
            let response = if transport_error {
                Err("Invalid engine response")
            } else {
                Ok(&failure)
            };
            assert!(flow.complete(&action, response, &view).is_err());
            assert!(!flow.pending());
            assert!(
                flow.complete(&pause, Ok(&reply(None, None)), &view)
                    .unwrap()
                    .action
                    .is_none()
            );
            assert_eq!(view.draft, Some(&document));
        }
    }

    #[test]
    fn save_first_switch_uses_both_post_save_revision_tokens() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let view = view(&document, &library);
        let mut flow = Workflow::default();
        let action = request_save_first(&mut flow, &view);
        let saved = saved(&document);
        let update = flow
            .complete(&action, Ok(&reply(Some(&saved), Some(9))), &view)
            .unwrap();
        assert_eq!(update.accepted.as_ref().unwrap().document, saved);
        let switch = update.action.unwrap();
        assert_switch(&switch, 9, 5);
        let opened = doc("other", 6);
        let update = flow
            .complete(&switch, Ok(&reply(Some(&opened), Some(10))), &view)
            .unwrap();
        assert_eq!(
            update.accepted.as_ref().unwrap().expected_draft.as_ref(),
            Some(&saved)
        );
        assert_eq!(update.accepted.unwrap().document, opened);
        assert!(update.action.is_none());
        assert!(!flow.pending());
    }

    #[test]
    fn missing_save_library_ack_refreshes_before_switch_and_rejects_stale_report() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let saved = saved(&document);
        for revision in [8, 9, 12] {
            let mut flow = Workflow::default();
            let action = request_save_first(&mut flow, &view(&document, &library));
            let update = flow
                .complete(
                    &action,
                    Ok(&reply(Some(&saved), None)),
                    &view(&document, &library),
                )
                .unwrap();
            assert!(update.accepted.is_some());
            let refresh = update.action.unwrap();
            assert!(matches!(refresh, Action::LibraryList));
            assert!(
                flow.complete(
                    &Action::Control(vec!["status".into()]),
                    Ok(&reply(None, None)),
                    &view(&saved, &library)
                )
                .unwrap()
                .action
                .is_none()
            );
            let response = reply(Some(&saved), Some(revision));
            let update = flow.complete(&refresh, Ok(&response), &view(&saved, &library));
            if revision == 8 {
                assert!(update.is_err());
                assert!(!flow.pending());
            } else {
                assert_switch(update.unwrap().action.as_ref().unwrap(), revision, 5);
            }
        }
    }

    #[test]
    fn valid_save_of_earlier_snapshot_advances_baseline_but_cancels_open() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut newer = document.clone();
        newer.layers.remove(1); // Deliberate Pi deletion must survive the old ack.
        let mut flow = Workflow::default();
        let action = request_save_first(&mut flow, &view(&document, &library));
        let saved = saved(&document);
        let update = flow
            .complete(
                &action,
                Ok(&reply(Some(&saved), Some(9))),
                &view(&newer, &library),
            )
            .unwrap();
        let accepted = update.accepted.unwrap();
        assert_eq!(accepted.document, saved);
        assert_eq!(accepted.expected_draft, Some(document));
        assert!(update.action.is_none());
        assert!(update.notice.unwrap().contains("newer draft retained"));
        assert!(!flow.pending());
        assert_eq!(newer.layers.len(), 1);
    }

    #[test]
    fn later_edits_during_library_refresh_also_abort_continuation() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut flow = Workflow::default();
        let action = request_save_first(&mut flow, &view(&document, &library));
        let saved = saved(&document);
        let refresh = flow
            .complete(
                &action,
                Ok(&reply(Some(&saved), None)),
                &view(&document, &library),
            )
            .unwrap()
            .action
            .unwrap();
        let mut newer = saved.clone();
        newer.name = "New edits".into();
        assert!(
            flow.complete(
                &refresh,
                Ok(&reply(Some(&saved), Some(9))),
                &view(&newer, &library)
            )
            .is_err()
        );
        assert!(!flow.pending());
    }

    #[test]
    fn unexpected_save_identity_content_revision_or_missing_ack_is_rejected() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        for invalid in 0..7 {
            let mut flow = Workflow::default();
            let action = flow
                .request_save(&view(&document, &library))
                .unwrap()
                .action
                .unwrap();
            let mut saved = saved(&document);
            match invalid {
                0 => saved.id = "unexpected".into(),
                1 => saved.name = "Unexpected content".into(),
                2 => saved.layers.clear(),
                3 => saved.revision = 7,
                _ => {}
            }
            let mut response = reply(Some(&saved), Some(9));
            if invalid == 4 {
                response.setup = None;
            }
            if invalid == 5 {
                response.setup.as_mut().unwrap().document = None;
            }
            if invalid == 6 {
                response.setup.as_mut().unwrap().revision += 1;
            }
            assert!(
                flow.complete(&action, Ok(&response), &view(&document, &library))
                    .is_err()
            );
            assert!(!flow.pending());
        }
    }

    #[test]
    fn bad_library_ack_or_missing_destination_does_not_hide_valid_save() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let saved = saved(&document);
        for invalid in 0..5 {
            let mut flow = Workflow::default();
            let action = request_save_first(&mut flow, &view(&document, &library));
            let mut response = reply(Some(&saved), Some(9));
            let report = response.library.as_mut().unwrap();
            match invalid {
                0 => report.schema_version = 2,
                1 => report.revision = 8,
                2 => report.current_id = Some("other".into()),
                3 => report.setups[0].name = "Wrong".into(),
                _ => report.setups.retain(|entry| entry.id != "other"),
            }
            let update = flow
                .complete(&action, Ok(&response), &view(&document, &library))
                .unwrap();
            assert_eq!(update.accepted.unwrap().document, saved);
            assert!(update.action.is_none());
            assert!(!flow.pending());
            assert!(update.notice.is_some());
        }
    }

    #[test]
    fn dirty_cancel_is_inert_and_discard_waits_for_remote_switch_success() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let view = view(&document, &library);
        let mut flow = Workflow::default();
        flow.request_destination(Destination::Saved("other".into()), &view)
            .unwrap();
        let cancelled = flow.resolve_dirty(DirtyChoice::Cancel, &view).unwrap();
        assert!(cancelled.action.is_none() && cancelled.new_draft.is_none());
        assert!(!flow.pending());
        flow.request_destination(Destination::Saved("other".into()), &view)
            .unwrap();
        let update = flow.resolve_dirty(DirtyChoice::Discard, &view).unwrap();
        assert!(update.accepted.is_none() && update.new_draft.is_none());
        let action = update.action.unwrap();
        assert_switch(&action, 8, 4);
        assert!(flow.complete(&action, Err("Disconnected"), &view).is_err());
        assert!(!flow.pending());
        assert_eq!(view.draft, Some(&document));
    }

    #[test]
    fn valid_save_without_paused_status_is_accepted_but_never_switches() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let saved = saved(&document);
        for missing in [false, true] {
            let mut flow = Workflow::default();
            let action = request_save_first(&mut flow, &view(&document, &library));
            let mut response = reply(Some(&saved), Some(9));
            if missing {
                response.status = None;
            } else {
                response.status.as_mut().unwrap().enabled = true;
            }
            let update = flow
                .complete(&action, Ok(&response), &view(&document, &library))
                .unwrap();
            assert_eq!(update.accepted.unwrap().document, saved);
            assert!(update.action.is_none());
            assert!(!flow.pending());
        }
    }

    #[test]
    fn save_before_local_starter_returns_ack_then_local_replacement_only() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let view = view(&document, &library);
        let candidate = doc("new-starter", 0);
        let mut flow = Workflow::default();
        flow.request_destination(Destination::NewDraft(candidate.clone()), &view)
            .unwrap();
        let action = flow
            .resolve_dirty(DirtyChoice::Save, &view)
            .unwrap()
            .action
            .unwrap();
        let saved = saved(&document);
        let update = flow
            .complete(&action, Ok(&reply(Some(&saved), None)), &view)
            .unwrap();
        assert_eq!(update.accepted.unwrap().document, saved);
        assert_eq!(update.new_draft, Some(candidate));
        assert!(update.action.is_none());
        assert!(!flow.pending());
    }

    #[test]
    fn invalid_switch_ack_never_replaces_a_dirty_draft() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let view = view(&document, &library);
        let mut flow = Workflow::default();
        flow.request_destination(Destination::Saved("other".into()), &view)
            .unwrap();
        let action = flow
            .resolve_dirty(DirtyChoice::Discard, &view)
            .unwrap()
            .action
            .unwrap();
        let unexpected = doc("unexpected", 5);
        assert!(
            flow.complete(&action, Ok(&reply(Some(&unexpected), Some(9))), &view)
                .is_err()
        );
        assert!(!flow.pending());
        assert_eq!(view.draft, Some(&document));
    }

    #[test]
    fn copy_uses_dirty_draft_fresh_identity_revision_zero_and_only_inactive_store() {
        let mut document = doc("daily", 4);
        document.layers.remove(1);
        let library = report(&doc("daily", 4), 8);
        let mut view = view(&document, &library);
        view.lighting = Lighting::Active;
        let flow = Workflow::default();
        let action = flow
            .save_copy(&view, "fresh-copy".into(), "My copy".into())
            .unwrap();
        match action {
            Action::LibraryUpdate {
                revision: 8,
                mutation: LibraryMutation::Store { document: copy },
            } => {
                assert_eq!(copy.id, "fresh-copy");
                assert_eq!(copy.revision, 0);
                assert_eq!(copy.name, "My copy");
                assert_eq!(copy.layers, document.layers);
            }
            _ => panic!("copy must only store an inactive document"),
        }
        assert!(
            flow.save_copy(&view, "daily".into(), "Collision".into())
                .is_err()
        );
        assert!(
            flow.save_copy(&view, "other".into(), "Collision".into())
                .is_err()
        );
        assert!(view.dirty);
        view.connected = false;
        assert!(
            flow.save_copy(&view, "offline".into(), "Offline".into())
                .is_err()
        );
    }

    #[test]
    fn new_starters_remain_local_and_save_uses_engine_base_not_draft_revision() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        view.connected = false;
        let candidate = doc("new-starter", 0);
        let mut flow = Workflow::default();
        flow.request_destination(Destination::NewDraft(candidate.clone()), &view)
            .unwrap();
        let update = flow.resolve_dirty(DirtyChoice::Discard, &view).unwrap();
        assert!(update.action.is_none());
        assert_eq!(update.new_draft, Some(candidate.clone()));
        let mut view = View {
            draft: Some(&candidate),
            connected: true,
            ..view
        };
        view.setup_revision = 4;
        let action = flow.request_save(&view).unwrap().action.unwrap();
        assert!(
            matches!(&action, Action::SetupReplace { revision: 4, document } if document.revision == 0)
        );
        let mut saved = candidate.clone();
        saved.revision = 5;
        let update = flow
            .complete(&action, Ok(&reply(Some(&saved), Some(9))), &view)
            .unwrap();
        assert_eq!(update.accepted.unwrap().document, saved);
    }

    #[test]
    fn offline_unknown_busy_and_changed_pause_snapshot_are_safe() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        let mut flow = Workflow::default();
        view.connected = false;
        assert!(flow.request_save(&view).is_err());
        view.connected = true;
        view.busy = true;
        assert!(flow.request_save(&view).is_err());
        view.busy = false;
        view.lighting = Lighting::Unknown;
        assert!(flow.request_save(&view).is_err());
        view.lighting = Lighting::Active;
        flow.request_save(&view).unwrap();
        let pause = flow.confirm_pause(&view).unwrap().action.unwrap();
        let mut newer = document.clone();
        newer.name = "Edited during pause".into();
        view.draft = Some(&newer);
        assert!(
            flow.complete(&pause, Ok(&reply(None, None)), &view)
                .is_err()
        );
        assert!(!flow.pending());
    }

    #[test]
    fn stale_completion_cannot_acknowledge_a_different_submitted_snapshot() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let view = view(&document, &library);
        let mut flow = Workflow::default();
        flow.request_save(&view).unwrap();
        let mut different = document.clone();
        different.name = "Old request".into();
        let stale = Action::SetupReplace {
            revision: 4,
            document: different.clone(),
        };
        let ignored = flow
            .complete(&stale, Ok(&reply(Some(&saved(&different)), Some(9))), &view)
            .unwrap();
        assert!(ignored.action.is_none() && ignored.accepted.is_none());
        assert!(flow.pending());
    }

    #[test]
    fn opening_active_saved_setup_requires_its_own_explicit_pause() {
        let document = doc("daily", 4);
        let library = report(&document, 8);
        let mut view = view(&document, &library);
        view.dirty = false;
        view.lighting = Lighting::Active;
        let mut flow = Workflow::default();
        flow.request_destination(Destination::Saved("other".into()), &view)
            .unwrap();
        assert_eq!(flow.prompt(), Some(Prompt::PauseAndOpen));
        let pause = flow.confirm_pause(&view).unwrap().action.unwrap();
        let update = flow
            .complete(&pause, Ok(&reply(None, None)), &view)
            .unwrap();
        assert_switch(update.action.as_ref().unwrap(), 8, 4);
    }
}
