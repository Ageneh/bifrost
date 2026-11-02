#![allow(clippy::future_not_send)]
//! A [`ServiceManager`] manages a collection of [`Service`] instances.
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::select;
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};
use uuid::Uuid;

use crate::error::{RunSvcError, SvcError, SvcResult};
use crate::rpc::RpcRequest;
use crate::runservice::StandardService;
use crate::serviceid::{IntoServiceId, ServiceId, ServiceName};
use crate::template::ServiceTemplate;
use crate::traits::{Service, ServiceRunner, ServiceState};

#[derive(Debug)]
pub struct ServiceInstance {
    tx: watch::Sender<ServiceState>,
    name: ServiceName,
    state: ServiceState,
    /// Why the service last failed, cleared when it runs again.
    last_error: Option<String>,
    abort_handle: AbortHandle,
}

pub type ServiceFunc = Box<
    dyn FnOnce(
            Uuid,
            watch::Receiver<ServiceState>,
            mpsc::UnboundedSender<ServiceEvent>,
        ) -> BoxFuture<'static, Result<(), RunSvcError>>
        + Send,
>;

#[derive(Debug, Clone)]
pub struct ServiceEvent {
    id: Uuid,
    state: ServiceState,
    error: Option<String>,
}

impl ServiceEvent {
    #[must_use]
    pub const fn new(id: Uuid, state: ServiceState) -> Self {
        Self {
            id,
            state,
            error: None,
        }
    }

    /// A transition that carries the reason the service failed.
    #[must_use]
    pub fn failed(id: Uuid, error: impl Into<String>) -> Self {
        Self {
            id,
            state: ServiceState::Failed,
            error: Some(error.into()),
        }
    }

    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    #[must_use]
    pub const fn state(&self) -> ServiceState {
        self.state
    }

    /// Why the service failed, when this transition failed it.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// A copy of this event with a known error attached, for the snapshot a
    /// new subscriber receives.
    #[must_use]
    fn with_error(mut self, error: Option<String>) -> Self {
        if self.error.is_none() {
            self.error = error;
        }
        self
    }
}

/// A request to a [`ServiceManager`]
pub enum SvmRequest {
    Stop(RpcRequest<ServiceId, SvcResult<Uuid>>),
    Start(RpcRequest<ServiceId, SvcResult<Uuid>>),
    Status(RpcRequest<ServiceId, SvcResult<ServiceState>>),
    LastError(RpcRequest<ServiceId, SvcResult<Option<String>>>),
    List(RpcRequest<(), Vec<(Uuid, ServiceName)>>),
    Resolve(RpcRequest<ServiceId, SvcResult<Uuid>>),
    LookupName(RpcRequest<ServiceId, SvcResult<ServiceName>>),
    Register(RpcRequest<(String, ServiceFunc), SvcResult<Uuid>>),
    RegisterTemplate(RpcRequest<(String, Box<dyn ServiceTemplate>), SvcResult<()>>),
    Subscribe(RpcRequest<mpsc::UnboundedSender<ServiceEvent>, SvcResult<Uuid>>),
    Unregister(RpcRequest<ServiceId, SvcResult<Uuid>>),
    Restart(RpcRequest<(ServiceId, Option<ServiceFunc>), SvcResult<Uuid>>),
    Shutdown(RpcRequest<(), ()>),
}

#[derive(Clone)]
pub struct SvmClient {
    tx: mpsc::UnboundedSender<SvmRequest>,
}

impl SvmClient {
    #[must_use]
    pub const fn new(tx: mpsc::UnboundedSender<SvmRequest>) -> Self {
        Self { tx }
    }

    pub async fn rpc<Q, A>(
        &mut self,
        func: impl FnOnce(RpcRequest<Q, A>) -> SvmRequest,
        args: Q,
    ) -> SvcResult<A> {
        let (rpc, rx) = RpcRequest::new(args);
        self.send(func(rpc))?;
        Ok(rx.await?)
    }

    fn send(&self, value: SvmRequest) -> SvcResult<()> {
        Ok(self.tx.send(value)?)
    }

    pub async fn register_service<S>(&mut self, name: impl AsRef<str>, svc: S) -> SvcResult<Uuid>
    where
        S: Service + 'static,
    {
        self.register(&name, StandardService::new(&name, svc)).await
    }

    pub async fn register_function<F, E>(
        &mut self,
        name: impl AsRef<str>,
        func: F,
    ) -> SvcResult<Uuid>
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Error + Send + 'static,
    {
        self.register(&name, StandardService::new(&name, Box::pin(func)))
            .await
    }

    pub async fn register<S>(&mut self, name: impl AsRef<str>, svc: S) -> SvcResult<Uuid>
    where
        S: ServiceRunner + Send + 'static,
    {
        let name = name.as_ref().to_string();
        self.rpc(
            SvmRequest::Register,
            (name, Box::new(|a, b, c| svc.run(a, b, c))),
        )
        .await?
    }

    pub async fn register_template(
        &mut self,
        name: impl AsRef<str>,
        generator: impl ServiceTemplate + 'static,
    ) -> SvcResult<()> {
        let name = name.as_ref().to_string();
        self.rpc(SvmRequest::RegisterTemplate, (name, Box::new(generator)))
            .await?
    }

    pub async fn start(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        self.rpc(SvmRequest::Start, id.service_id()).await?
    }

    pub async fn stop(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        self.rpc(SvmRequest::Stop, id.service_id()).await?
    }

    pub async fn resolve(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        self.rpc(SvmRequest::Resolve, id.service_id()).await?
    }

    pub async fn lookup_name(&mut self, id: impl IntoServiceId) -> SvcResult<ServiceName> {
        self.rpc(SvmRequest::LookupName, id.service_id()).await?
    }

    pub async fn subscribe(&mut self) -> SvcResult<(Uuid, mpsc::UnboundedReceiver<ServiceEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();

        let uuid = self.rpc(SvmRequest::Subscribe, tx).await??;

        Ok((uuid, rx))
    }

    pub async fn wait_for_state(
        &mut self,
        handle: impl IntoServiceId,
        expected: ServiceState,
    ) -> SvcResult<()> {
        let svc_id = self.resolve(&handle).await?;

        let (_cid, mut channel) = self.subscribe().await?;

        while let Some(msg) = channel.recv().await {
            if msg.id == svc_id {
                if msg.state == expected {
                    return Ok(());
                }

                if msg.state == ServiceState::Failed {
                    return Err(SvcError::ServiceFailed);
                }
            }
        }

        Err(SvcError::Shutdown)
    }

    pub async fn wait_for_start(
        &mut self,
        handle: impl IntoServiceId + Send + 'static,
    ) -> SvcResult<()> {
        self.wait_for_state(handle, ServiceState::Running).await
    }

    pub async fn wait_for_stop(
        &mut self,
        handle: impl IntoServiceId + Send + 'static,
    ) -> SvcResult<()> {
        self.wait_for_state(handle, ServiceState::Stopped).await
    }

    pub async fn status(
        &mut self,
        id: impl IntoServiceId + Send + 'static,
    ) -> SvcResult<ServiceState> {
        self.rpc(SvmRequest::Status, id.service_id()).await?
    }

    /// Why the service last failed, if it did and has not run since.
    pub async fn last_error(
        &mut self,
        id: impl IntoServiceId + Send + 'static,
    ) -> SvcResult<Option<String>> {
        self.rpc(SvmRequest::LastError, id.service_id()).await?
    }

    pub async fn list(&mut self) -> SvcResult<Vec<(Uuid, ServiceName)>> {
        self.rpc(SvmRequest::List, ()).await
    }

    pub async fn shutdown(&mut self) -> SvcResult<()> {
        self.rpc(SvmRequest::Shutdown, ()).await
    }

    /// Abort a service and forget it, so its name can be registered again.
    pub async fn unregister(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        self.rpc(SvmRequest::Unregister, id.service_id()).await?
    }

    /// Replace a templated service instance with a freshly generated one,
    /// and start it.
    pub async fn restart(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        self.rpc(SvmRequest::Restart, (id.service_id(), None))
            .await?
    }

    /// Replace a service with `svc` under the same name, and start it.
    pub async fn restart_with<S>(&mut self, id: impl IntoServiceId, svc: S) -> SvcResult<Uuid>
    where
        S: Service + 'static,
    {
        let func = StandardService::new(id.to_string(), svc).boxed();
        self.rpc(SvmRequest::Restart, (id.service_id(), Some(func)))
            .await?
    }
}

impl Debug for SvmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop(arg0) => f.debug_tuple("Stop").field(arg0).finish(),
            Self::Start(arg0) => f.debug_tuple("Start").field(arg0).finish(),
            Self::Status(arg0) => f.debug_tuple("Status").field(arg0).finish(),
            Self::LastError(arg0) => f.debug_tuple("LastError").field(arg0).finish(),
            Self::List(arg0) => f.debug_tuple("List").field(arg0).finish(),
            Self::Register(_arg0) => f.debug_tuple("Register").field(&"<service>").finish(),
            Self::RegisterTemplate(_arg0) => f
                .debug_tuple("RegisterTemplate")
                .field(&"<service>")
                .finish(),
            Self::Resolve(arg0) => f.debug_tuple("Resolve").field(arg0).finish(),
            Self::LookupName(arg0) => f.debug_tuple("ResolveName").field(arg0).finish(),
            Self::Subscribe(_arg0) => f.debug_tuple("Subscribe").finish(),
            Self::Unregister(arg0) => f.debug_tuple("Unregister").field(arg0).finish(),
            Self::Restart(arg0) => f.debug_tuple("Restart").field(&arg0.data().0).finish(),
            Self::Shutdown(_arg0) => f.debug_tuple("Shutdown").finish(),
        }
    }
}

pub struct ServiceManager {
    control_rx: mpsc::UnboundedReceiver<SvmRequest>,
    control_tx: mpsc::UnboundedSender<SvmRequest>,
    service_rx: mpsc::UnboundedReceiver<ServiceEvent>,
    service_tx: mpsc::UnboundedSender<ServiceEvent>,
    subscribers: BTreeMap<Uuid, mpsc::UnboundedSender<ServiceEvent>>,
    svcs: BTreeMap<Uuid, ServiceInstance>,
    names: BTreeMap<ServiceName, Uuid>,
    tasks: JoinSet<Result<(), RunSvcError>>,
    templates: BTreeMap<String, Box<dyn ServiceTemplate>>,
    shutdown: bool,
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceManager {
    #[must_use]
    pub fn new() -> Self {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (service_tx, service_rx) = mpsc::unbounded_channel();
        Self {
            control_tx,
            control_rx,
            service_tx,
            service_rx,
            subscribers: BTreeMap::new(),
            svcs: BTreeMap::new(),
            names: BTreeMap::new(),
            tasks: JoinSet::new(),
            templates: BTreeMap::new(),
            shutdown: false,
        }
    }

    /// Daemonize the [`ServiceManager`], returning a (clonable) [`SvmClient`] as
    /// well as a [`JoinHandle`] used to control the service manager task
    /// itself.
    #[must_use]
    pub fn daemonize(self) -> (SvmClient, JoinHandle<SvcResult<()>>) {
        let client = self.client();
        let fut = tokio::task::spawn(self.run());
        (client, fut)
    }

    /// Convenience function to create and daemonize a [`ServiceManager`].
    #[must_use]
    pub fn spawn() -> (SvmClient, JoinHandle<SvcResult<()>>) {
        Self::new().daemonize()
    }

    /// Create a new [`SvmClient`] connected to this service manager.
    #[must_use]
    pub fn client(&self) -> SvmClient {
        SvmClient::new(self.handle())
    }

    fn handle(&self) -> mpsc::UnboundedSender<SvmRequest> {
        self.control_tx.clone()
    }

    fn register(&mut self, name: ServiceName, svc: ServiceFunc) -> SvcResult<Uuid> {
        if self.names.contains_key(&name) {
            return Err(SvcError::ServiceAlreadyExists(name));
        }

        let (tx, rx) = watch::channel(ServiceState::Registered);
        let id = Uuid::new_v4();

        let abort_handle = self.tasks.spawn((svc)(id, rx, self.service_tx.clone()));

        let rec = ServiceInstance {
            tx,
            name: name.clone(),
            state: ServiceState::Registered,
            last_error: None,
            abort_handle,
        };

        self.svcs.insert(id, rec);
        self.names.insert(name, id);

        Ok(id)
    }

    fn list(&self) -> impl Iterator<Item = &Uuid> {
        self.svcs.keys()
    }

    fn resolve(&self, handle: impl IntoServiceId) -> SvcResult<Uuid> {
        let id = handle.service_id();
        match &id {
            ServiceId::Name(name) => self
                .names
                .get(name)
                .ok_or_else(|| SvcError::ServiceNotFound(id))
                .copied(),
            ServiceId::Id(uuid) => {
                if self.svcs.contains_key(uuid) {
                    Ok(*uuid)
                } else {
                    Err(SvcError::ServiceNotFound(id))
                }
            }
        }
    }

    fn remove(&mut self, handle: &ServiceId) -> SvcResult<()> {
        let id = self.resolve(handle)?;
        self.svcs.remove(&id);
        self.names.retain(|_, v| *v != id);

        Ok(())
    }

    fn abort(&mut self, id: &ServiceId) -> SvcResult<()> {
        let svc = self.get(id)?;

        svc.abort_handle.abort();

        self.remove(id)
    }

    fn get(&self, svc: impl IntoServiceId) -> SvcResult<&ServiceInstance> {
        let id = self.resolve(svc)?;
        Ok(&self.svcs[&id])
    }

    fn start(&mut self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        let id = id.service_id();

        // if the service is known, attempt to start it
        if let Ok(svc) = self.get(&id) {
            // a repeated start must not wake the service: its run loop
            // cancels the running future on any change, and a service that
            // moves its work into that future (an HTTP server) loses it
            if *svc.tx.borrow() != ServiceState::Running {
                log::debug!("Starting service: {id} {}", &svc.name);
                svc.tx.send(ServiceState::Running)?;
            }
            return self.resolve(&id);
        }

        // ..else, check if it's a named instance
        let ServiceId::Name(svc_name) = &id else {
            return Err(SvcError::ServiceNotFound(id));
        };

        let svc = self.generate(svc_name)?;

        self.register(svc_name.clone(), svc)
    }

    /// Generate a service instance from its template.
    fn generate(&self, svc_name: &ServiceName) -> SvcResult<ServiceFunc> {
        let not_found = || SvcError::ServiceNotFound(ServiceId::Name(svc_name.clone()));

        let inst = svc_name.instance().ok_or_else(not_found)?;
        let tmpl = self.templates.get(svc_name.name()).ok_or_else(not_found)?;

        let inner = tmpl.generate(inst.to_string())?;
        Ok(StandardService::new(svc_name.name(), inner).boxed())
    }

    fn unregister(&mut self, id: &ServiceId) -> SvcResult<Uuid> {
        let uuid = self.resolve(id)?;
        log::debug!("Unregistering service: {uuid} {}", self.svcs[&uuid].name);
        self.abort(id)?;
        Ok(uuid)
    }

    /// Unregister followed by re-register. The replacement is built before
    /// the old service is removed, so a failing template leaves it in place.
    fn restart(&mut self, id: &ServiceId, svc: Option<ServiceFunc>) -> SvcResult<Uuid> {
        let name = self.get(id)?.name.clone();
        let svc = match svc {
            Some(svc) => svc,
            None => self.generate(&name)?,
        };

        self.unregister(id)?;
        let uuid = self.register(name, svc)?;
        self.svcs[&uuid].tx.send(ServiceState::Running)?;

        Ok(uuid)
    }

    fn stop(&self, id: impl IntoServiceId) -> SvcResult<Uuid> {
        let id = self.resolve(id)?;

        if self.svcs[&id].state == ServiceState::Stopped {
            return Ok(id);
        }

        log::debug!("Stopping service: {id} {}", self.svcs[&id].name);
        self.get(id)
            .and_then(|svc| Ok(svc.tx.send(ServiceState::Stopped)?))?;

        Ok(id)
    }

    fn notify_subscribers(&mut self, event: ServiceEvent) {
        let mut failed = vec![];
        for (key, sub) in &self.subscribers {
            log::trace!("UPDATE: [sub-{key}] {} -> {:?}", &event.id, &event.state);
            if sub.send(event.clone()).is_err() {
                failed.push(*key);
            }
        }
        if !failed.is_empty() {
            self.subscribers.retain(|k, _| !failed.contains(k));
        }
    }

    async fn next_event(&mut self) -> SvcResult<()> {
        tokio::select! {
            event = self.control_rx.recv() => self.handle_svm_request(event.ok_or(SvcError::Shutdown)?).await,
            event = self.service_rx.recv() => {
                self.handle_service_event(event.ok_or(SvcError::Shutdown)?);
                Ok(())
            },
        }
    }

    fn handle_service_event(&mut self, event: ServiceEvent) {
        // an unregistered service can still have events in flight
        let Some(svc) = self.svcs.get_mut(&event.id) else {
            log::trace!("[{}] Ignoring event for unregistered service", event.id);
            return;
        };
        svc.state = event.state;
        // a service that runs again has recovered; keep only a live reason
        svc.last_error = match event.state {
            ServiceState::Running => None,
            _ => event.error.clone().or_else(|| svc.last_error.clone()),
        };
        log::trace!(
            "[{}] [{}] Service is now {:?}",
            svc.name,
            event.id,
            event.state
        );
        self.notify_subscribers(event);
    }

    async fn handle_svm_request(&mut self, upd: SvmRequest) -> SvcResult<()> {
        match upd {
            SvmRequest::Start(rpc) => rpc.respond(|id| self.start(&id)),

            SvmRequest::Stop(rpc) => rpc.respond(|id| self.stop(&id)),

            SvmRequest::Status(rpc) => rpc.respond(|id| Ok(self.get(&id)?.state)),

            SvmRequest::LastError(rpc) => {
                rpc.respond(|id| Ok(self.get(&id)?.last_error.clone()));
            }

            SvmRequest::List(rpc) => rpc.respond(|()| {
                let mut res = vec![];

                for (name, id) in &self.names {
                    res.push((*id, name.clone()));
                }
                res
            }),

            SvmRequest::Register(rpc) => {
                rpc.respond(|(name, svc)| self.register(ServiceName::from(name), svc));
            }

            SvmRequest::RegisterTemplate(rpc) => rpc.respond(|(name, tmpl)| {
                self.templates.insert(name, tmpl);
                Ok(())
            }),

            SvmRequest::Resolve(rpc) => rpc.respond(|id| self.resolve(&id)),

            SvmRequest::LookupName(rpc) => rpc.respond(|id| Ok(self.get(&id)?.name.clone())),

            SvmRequest::Subscribe(rpc) => {
                for (id, svc) in &self.svcs {
                    rpc.data().send(
                        ServiceEvent::new(*id, svc.state).with_error(svc.last_error.clone()),
                    )?;
                }

                rpc.respond(|tx| {
                    let uuid = Uuid::new_v4();
                    self.subscribers.insert(uuid, tx);

                    Ok(uuid)
                });
            }

            SvmRequest::Unregister(rpc) => rpc.respond(|id| self.unregister(&id)),

            SvmRequest::Restart(rpc) => rpc.respond(|(id, svc)| self.restart(&id, svc)),

            SvmRequest::Shutdown(rpc) => {
                log::info!("Service manager shutting down..");
                let ids: Vec<Uuid> = self.list().copied().collect();

                self.stop_multiple(&ids)?;

                select! {
                    Ok(()) = Box::pin(self.wait_for_multiple(&ids, ServiceState::Stopped)) => {}
                    () = tokio::time::sleep(Duration::from_secs(3)) => {
                        log::error!("Service shutdown timed out, aborting tasks..");

                        for id in &ids {
                            let si = self.get(id)?;
                            log::error!("  ..aborting {id}: {si:?}");
                            self.abort(&ServiceId::from(*id))?;
                        }
                    }
                }
                log::debug!("All services stopped.");
                self.shutdown = true;
                rpc.respond(|()| ());
            }
        }

        Ok(())
    }

    fn stop_multiple(&self, handles: &[impl IntoServiceId]) -> SvcResult<()> {
        let ids = self.resolve_multiple(handles)?;
        for id in ids {
            self.stop(id)?;
        }

        Ok(())
    }

    fn resolve_multiple(&self, handles: &[impl IntoServiceId]) -> SvcResult<BTreeSet<Uuid>> {
        let res = BTreeSet::from_iter(
            handles
                .iter()
                .map(|id| self.resolve(id))
                .collect::<Result<Vec<Uuid>, SvcError>>()?,
        );

        Ok(res)
    }

    async fn wait_for_multiple(
        &mut self,
        handles: &[impl IntoServiceId],
        target: ServiceState,
    ) -> SvcResult<()> {
        let mut missing = self.resolve_multiple(handles)?;
        let mut done = BTreeSet::new();

        loop {
            for m in &missing {
                let state = self.get(*m)?.state;

                if state == ServiceState::Failed && target != ServiceState::Stopped {
                    return Err(SvcError::ServiceFailed);
                }

                if state == target {
                    done.insert(*m);
                }
            }

            missing.retain(|f| !done.contains(f));

            if missing.is_empty() {
                return Ok(());
            }

            self.next_event().await?;
        }
    }

    pub async fn run(mut self) -> SvcResult<()> {
        while !self.shutdown {
            self.next_event().await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::error::{SvcError, SvcResult};
    use crate::manager::{ServiceManager, SvmClient};
    use crate::serviceid::{ServiceId, ServiceName};
    use crate::traits::{BoxDynService, Service, ServiceState};

    /// A service that runs until it is stopped or aborted.
    struct Idle;

    #[async_trait::async_trait]
    impl Service for Idle {
        type Error = Infallible;

        async fn run(&mut self) -> Result<(), Infallible> {
            futures::future::pending().await
        }
    }

    async fn names(client: &mut SvmClient) -> Vec<String> {
        let mut names: Vec<String> = client
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|(_, name)| name.to_string())
            .collect();
        names.sort();
        names
    }

    async fn counting_template(client: &mut SvmClient) -> Arc<AtomicUsize> {
        let generated = Arc::new(AtomicUsize::new(0));
        let counter = generated.clone();
        client
            .register_template("tmpl", move |_instance: String| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Idle.boxed() as BoxDynService)
            })
            .await
            .unwrap();
        generated
    }

    #[tokio::test]
    async fn unregister_removes_service_and_frees_its_name() -> SvcResult<()> {
        let (mut client, _future) = ServiceManager::spawn();

        let first = client.register_service("idle", Idle).await?;
        client.start("idle").await?;
        client.wait_for_start("idle").await?;

        assert_eq!(client.unregister("idle").await?, first);
        assert_eq!(names(&mut client).await, Vec::<String>::new());
        assert!(matches!(
            client.status("idle").await,
            Err(SvcError::ServiceNotFound(_))
        ));

        let second = client.register_service("idle", Idle).await?;
        assert_ne!(first, second);
        client.start("idle").await?;
        client.wait_for_start("idle").await?;

        Ok(())
    }

    /// A service that, like an HTTP server, moves its work into `run` and
    /// completes at once when `run` is entered again without a new `start`.
    struct TakesWork {
        work: Option<()>,
    }

    #[async_trait::async_trait]
    impl Service for TakesWork {
        type Error = Infallible;

        async fn start(&mut self) -> Result<(), Infallible> {
            self.work = Some(());
            Ok(())
        }

        async fn run(&mut self) -> Result<(), Infallible> {
            if self.work.take().is_some() {
                futures::future::pending::<()>().await;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn repeated_start_keeps_service_running() -> SvcResult<()> {
        let (mut client, _future) = ServiceManager::spawn();

        client
            .register_service("server", TakesWork { work: None })
            .await?;
        client.start("server").await?;
        client.wait_for_start("server").await?;

        client.start("server").await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(client.status("server").await?, ServiceState::Running);

        Ok(())
    }

    #[tokio::test]
    async fn unregister_unknown_service_fails() {
        let (mut client, _future) = ServiceManager::spawn();

        assert!(matches!(
            client.unregister("missing").await,
            Err(SvcError::ServiceNotFound(_))
        ));
    }

    #[tokio::test]
    async fn restart_regenerates_template_instance() -> SvcResult<()> {
        let (mut client, _future) = ServiceManager::spawn();
        let generated = counting_template(&mut client).await;
        let instance = ServiceId::instance("tmpl", "one");

        // a templated instance is registered by the first start, run by the second
        let first = client.start(&instance).await?;
        client.start(first).await?;
        client.wait_for_start(first).await?;

        let second = client.restart(&instance).await?;
        client.wait_for_start(second).await?;

        assert_ne!(first, second);
        assert_eq!(generated.load(Ordering::SeqCst), 2);
        assert_eq!(names(&mut client).await, ["tmpl@one"]);

        Ok(())
    }

    #[tokio::test]
    async fn restart_with_replaces_service() -> SvcResult<()> {
        let (mut client, _future) = ServiceManager::spawn();

        let first = client.register_service("idle", Idle).await?;
        client.start(first).await?;
        client.wait_for_start(first).await?;

        let second = client.restart_with("idle", Idle).await?;
        client.wait_for_start(second).await?;

        assert_ne!(first, second);
        assert_eq!(client.resolve("idle").await?, second);

        Ok(())
    }

    #[tokio::test]
    async fn restart_without_template_keeps_service() -> SvcResult<()> {
        let (mut client, _future) = ServiceManager::spawn();

        let id = client.register_service("idle", Idle).await?;

        assert!(matches!(
            client.restart("idle").await,
            Err(SvcError::ServiceNotFound(_))
        ));
        assert_eq!(client.resolve("idle").await?, id);
        assert_eq!(client.lookup_name(id).await?, ServiceName::from("idle"));

        Ok(())
    }

    #[tokio::test]
    async fn shutdown_after_unregister_completes() -> SvcResult<()> {
        let (mut client, future) = ServiceManager::spawn();

        client.register_service("keep", Idle).await?;
        client.register_service("drop", Idle).await?;
        client.start("keep").await?;
        client.start("drop").await?;
        client.wait_for_start("drop").await?;
        client.unregister("drop").await?;

        client.shutdown().await?;
        future.await??;

        Ok(())
    }
}
