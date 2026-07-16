use std::{
    collections::VecDeque,
    future::pending,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use http::{HeaderMap, StatusCode};
use time::OffsetDateTime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::Instant,
};

use super::{
    AddressPolicy,
    fetch::{
        BodyError, ExecuteError, HttpBody, HttpExecuteRequest, HttpExecutor, HttpResponse,
        SnapshotProvider,
    },
    resolver::{
        DnsResolveError, DnsResolver, Nat64Discovery, Nat64DiscoveryError, Nat64PrefixDiscovery,
        Nat64Snapshot,
    },
};

pub(super) type EventLog = Arc<Mutex<Vec<&'static str>>>;

pub(super) fn event_log() -> EventLog {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) struct FakeDnsResolver {
    replies: Mutex<VecDeque<DnsReply>>,
    calls: Mutex<Vec<String>>,
    events: Option<EventLog>,
}

pub(super) struct DnsReply {
    delay: Duration,
    result: Result<Vec<IpAddr>, DnsResolveError>,
}

impl DnsReply {
    pub(super) fn addresses(addresses: Vec<IpAddr>) -> Self {
        Self {
            delay: Duration::ZERO,
            result: Ok(addresses),
        }
    }

    pub(super) fn delayed(delay: Duration, addresses: Vec<IpAddr>) -> Self {
        Self {
            delay,
            result: Ok(addresses),
        }
    }
}

impl FakeDnsResolver {
    pub(super) fn new(replies: Vec<DnsReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
            events: None,
        }
    }

    pub(super) fn with_events(replies: Vec<DnsReply>, events: EventLog) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
            events: Some(events),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl DnsResolver for FakeDnsResolver {
    async fn resolve(
        &self,
        host: &str,
        _deadline: Instant,
    ) -> Result<Vec<IpAddr>, DnsResolveError> {
        if let Some(events) = &self.events {
            events.lock().unwrap().push("dns");
        }
        self.calls.lock().unwrap().push(host.to_owned());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted DNS reply");
        if !reply.delay.is_zero() {
            tokio::time::sleep(reply.delay).await;
        }
        reply.result
    }
}

pub(super) struct FakeNat64Discovery {
    replies: Mutex<VecDeque<Result<Nat64Discovery, Nat64DiscoveryError>>>,
    calls: Mutex<usize>,
    events: Option<EventLog>,
}

impl FakeNat64Discovery {
    pub(super) fn new(replies: Vec<Result<Nat64Discovery, Nat64DiscoveryError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(0),
            events: None,
        }
    }

    pub(super) fn with_events(
        replies: Vec<Result<Nat64Discovery, Nat64DiscoveryError>>,
        events: EventLog,
    ) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(0),
            events: Some(events),
        }
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Nat64PrefixDiscovery for FakeNat64Discovery {
    async fn discover(&self, _deadline: Instant) -> Result<Nat64Discovery, Nat64DiscoveryError> {
        if let Some(events) = &self.events {
            events.lock().unwrap().push("discovery");
        }
        *self.calls.lock().unwrap() += 1;
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted NAT64 discovery reply")
    }
}

pub(super) struct ScriptedSnapshots {
    snapshots: Mutex<VecDeque<Arc<Nat64Snapshot>>>,
}

impl ScriptedSnapshots {
    pub(super) fn stable(snapshot: Arc<Nat64Snapshot>) -> Self {
        Self {
            snapshots: Mutex::new(VecDeque::from([snapshot])),
        }
    }

    pub(super) fn sequence(snapshots: Vec<Arc<Nat64Snapshot>>) -> Self {
        Self {
            snapshots: Mutex::new(snapshots.into()),
        }
    }
}

#[async_trait]
impl SnapshotProvider for ScriptedSnapshots {
    async fn current(
        &self,
        _total_deadline: Instant,
    ) -> Result<Arc<Nat64Snapshot>, Nat64DiscoveryError> {
        let mut snapshots = self.snapshots.lock().unwrap();
        if snapshots.len() > 1 {
            Ok(snapshots.pop_front().unwrap())
        } else {
            Ok(snapshots.front().expect("scripted snapshot").clone())
        }
    }
}

pub(super) fn snapshot(generation: u64) -> Arc<Nat64Snapshot> {
    Arc::new(Nat64Snapshot {
        generation,
        valid_until: None,
        address_policy: AddressPolicy::public_only(),
    })
}

pub(super) struct StalledHttpServer {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl StalledHttpServer {
    pub(super) async fn start(response_head: &'static [u8]) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(response_head).await.unwrap();
            pending::<()>().await;
        });
        tokio::task::yield_now().await;
        Self { address, task }
    }

    pub(super) const fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for StalledHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy)]
pub(super) enum PeerSpec {
    ApprovedFirst,
    Missing,
    Fixed(SocketAddr),
}

pub(super) struct ResponseSpec {
    pub(super) delay: Duration,
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) peer: PeerSpec,
    pub(super) body: ScriptedBody,
}

impl ResponseSpec {
    pub(super) fn new(status: StatusCode) -> Self {
        Self {
            delay: Duration::ZERO,
            status,
            headers: HeaderMap::new(),
            peer: PeerSpec::ApprovedFirst,
            body: ScriptedBody::empty(),
        }
    }
}

pub(super) struct ScriptedExecutor {
    responses: Mutex<VecDeque<ResponseSpec>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

#[derive(Clone)]
pub(super) struct CapturedRequest {
    pub(super) url: String,
    pub(super) host: String,
    pub(super) approved: Vec<SocketAddr>,
    pub(super) if_none_match: Option<Vec<u8>>,
    pub(super) if_modified_since: Option<Vec<u8>>,
    pub(super) request_timeout: Duration,
}

impl ScriptedExecutor {
    pub(super) fn new(responses: Vec<ResponseSpec>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl HttpExecutor for ScriptedExecutor {
    async fn execute(&self, request: HttpExecuteRequest) -> Result<HttpResponse, ExecuteError> {
        self.requests.lock().unwrap().push(CapturedRequest {
            url: request.url.clone(),
            host: request.host.clone(),
            approved: request.approved.clone(),
            if_none_match: request
                .if_none_match
                .as_ref()
                .map(|value| value.as_bytes().to_vec()),
            if_modified_since: request
                .if_modified_since
                .as_ref()
                .map(|value| value.as_bytes().to_vec()),
            request_timeout: request.request_timeout,
        });
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted HTTP response");
        if !response.delay.is_zero() {
            tokio::time::sleep(response.delay).await;
        }
        let peer = match response.peer {
            PeerSpec::ApprovedFirst => request.approved.first().copied(),
            PeerSpec::Missing => None,
            PeerSpec::Fixed(peer) => Some(peer),
        };
        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            peer,
            received_at: OffsetDateTime::now_utc(),
            body: Box::new(response.body),
        })
    }
}

pub(super) struct ScriptedBody {
    steps: VecDeque<BodyStep>,
    polls: Arc<Mutex<usize>>,
}

pub(super) struct BodyStep {
    delay: Duration,
    result: Result<Option<Vec<u8>>, ()>,
}

impl BodyStep {
    pub(super) fn chunk(bytes: Vec<u8>) -> Self {
        Self {
            delay: Duration::ZERO,
            result: Ok(Some(bytes)),
        }
    }

    pub(super) fn delayed_chunk(delay: Duration, bytes: Vec<u8>) -> Self {
        Self {
            delay,
            result: Ok(Some(bytes)),
        }
    }

    pub(super) fn end() -> Self {
        Self {
            delay: Duration::ZERO,
            result: Ok(None),
        }
    }

    pub(super) fn delayed_end(delay: Duration) -> Self {
        Self {
            delay,
            result: Ok(None),
        }
    }
}

impl ScriptedBody {
    pub(super) fn empty() -> Self {
        Self::new(vec![BodyStep::end()]).0
    }

    pub(super) fn new(steps: Vec<BodyStep>) -> (Self, Arc<Mutex<usize>>) {
        let polls = Arc::new(Mutex::new(0));
        (
            Self {
                steps: steps.into(),
                polls: polls.clone(),
            },
            polls,
        )
    }
}

#[async_trait]
impl HttpBody for ScriptedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BodyError> {
        *self.polls.lock().unwrap() += 1;
        let step = self.steps.pop_front().expect("scripted body step");
        if !step.delay.is_zero() {
            tokio::time::sleep(step.delay).await;
        }
        step.result.map_err(|()| BodyError::Other)
    }
}
