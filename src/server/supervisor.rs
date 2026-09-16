use std::collections::HashMap;
use std::future::Future;

use anyhow::Result;
use tokio::task::{Id, JoinSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Ended {
    Returned,
    Failed(String),
    Panicked(String),
}

impl Ended {
    pub(crate) fn is_fault(&self) -> bool {
        !matches!(self, Self::Returned)
    }

    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Returned => "returned".to_owned(),
            Self::Failed(error) => format!("failed: {error}"),
            Self::Panicked(payload) => format!("panicked: {payload}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Stop {
    Signal(String),
    Task { name: String, ended: Ended },
    Drained,
}

#[derive(Default)]
pub(crate) struct Supervisor {
    tasks: JoinSet<Result<()>>,
    names: HashMap<Id, String>,
}

impl Supervisor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn spawn(
        &mut self,
        name: impl Into<String>,
        task: impl Future<Output = Result<()>> + Send + 'static,
    ) {
        let handle = self.tasks.spawn(task);
        self.names.insert(handle.id(), name.into());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    fn name_of(&self, id: Id) -> String {
        self.names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "unnamed task".to_owned())
    }

    pub(crate) async fn next_ended(&mut self) -> Option<(String, Ended)> {
        let joined = self.tasks.join_next_with_id().await?;
        Some(match joined {
            Ok((id, Ok(()))) => (self.name_of(id), Ended::Returned),
            Ok((id, Err(error))) => (self.name_of(id), Ended::Failed(format!("{error:#}"))),
            Err(error) => {
                let name = self.name_of(error.id());
                if error.is_panic() {
                    let payload = error
                        .into_panic()
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_owned())
                        .unwrap_or_else(|| "panic payload was not a string".to_owned());
                    (name, Ended::Panicked(payload))
                } else {
                    (name, Ended::Failed("cancelled".to_owned()))
                }
            }
        })
    }

    pub(crate) async fn drain_within(mut self, grace: std::time::Duration) -> Vec<(String, Ended)> {
        let mut ended = Vec::new();
        let deadline = tokio::time::Instant::now() + grace;

        loop {
            match tokio::time::timeout_at(deadline, self.next_ended()).await {
                Ok(Some(next)) => ended.push(next),
                Ok(None) => break,
                Err(_) => {
                    self.abort_all();
                    while let Some(next) = self.next_ended().await {
                        ended.push(next);
                    }
                    break;
                }
            }
        }

        ended
    }

    fn abort_all(&mut self) {
        self.tasks.abort_all();
    }
}

pub(crate) async fn wait_for_stop(
    supervisor: &mut Supervisor,
    signal: impl Future<Output = String>,
) -> Stop {
    if supervisor.is_empty() {
        return Stop::Drained;
    }

    tokio::select! {
        cause = signal => Stop::Signal(cause),
        ended = supervisor.next_ended() => match ended {
            Some((name, ended)) => Stop::Task { name, ended },
            None => Stop::Drained,
        },
    }
}

#[cfg(test)]
#[path = "../../tests/server/supervisor.rs"]
mod tests;
