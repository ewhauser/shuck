use std::cell::{Cell, OnceCell, RefCell};
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use lsp_server::RequestId;
use rustc_hash::FxHashMap;

use crate::session::client::ClientResponseHandler;

pub(crate) struct RequestQueue {
    incoming: Incoming,
    outgoing: Outgoing,
}

impl RequestQueue {
    pub(super) fn new() -> Self {
        Self {
            incoming: Incoming::default(),
            outgoing: Outgoing::default(),
        }
    }

    pub(crate) fn outgoing_mut(&mut self) -> &mut Outgoing {
        &mut self.outgoing
    }

    pub(crate) fn outgoing(&self) -> &Outgoing {
        &self.outgoing
    }

    pub(crate) fn incoming(&self) -> &Incoming {
        &self.incoming
    }

    pub(crate) fn incoming_mut(&mut self) -> &mut Incoming {
        &mut self.incoming
    }
}

#[derive(Default, Debug)]
pub(crate) struct Incoming {
    pending: FxHashMap<RequestId, PendingRequest>,
}

impl Incoming {
    pub(crate) fn register(&mut self, request_id: RequestId, method: String) {
        self.pending.insert(request_id, PendingRequest::new(method));
    }

    pub(super) fn cancel(&mut self, request_id: &RequestId) -> Option<String> {
        self.pending.remove(request_id).map(|mut pending| {
            if let Some(cancellation_token) = pending.cancellation_token.take() {
                cancellation_token.cancel();
            }
            pending.method
        })
    }

    pub(crate) fn cancellation_token(
        &self,
        request_id: &RequestId,
    ) -> Option<RequestCancellationToken> {
        let pending = self.pending.get(request_id)?;
        Some(RequestCancellationToken::clone(
            pending
                .cancellation_token
                .get_or_init(RequestCancellationToken::default),
        ))
    }

    pub(crate) fn complete(&mut self, request_id: &RequestId) -> Option<(Instant, String)> {
        self.pending
            .remove(request_id)
            .map(|pending| (pending.start_time, pending.method))
    }
}

#[derive(Debug)]
struct PendingRequest {
    start_time: Instant,
    method: String,
    cancellation_token: OnceCell<RequestCancellationToken>,
}

impl PendingRequest {
    fn new(method: String) -> Self {
        Self {
            start_time: Instant::now(),
            method,
            cancellation_token: OnceCell::new(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct RequestCancellationToken(Arc<AtomicBool>);

impl RequestCancellationToken {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn clone(this: &Self) -> Self {
        Self(this.0.clone())
    }
}

#[derive(Default)]
pub(crate) struct Outgoing {
    next_request_id: Cell<i32>,
    response_handlers: RefCell<FxHashMap<RequestId, ClientResponseHandler>>,
}

impl Outgoing {
    pub(crate) fn register(&self, handler: ClientResponseHandler) -> RequestId {
        let id = self.next_request_id.get();
        self.next_request_id.set(id + 1);
        self.response_handlers
            .borrow_mut()
            .insert(id.into(), handler);
        id.into()
    }

    pub(crate) fn complete(&mut self, request_id: &RequestId) -> Option<ClientResponseHandler> {
        self.response_handlers.get_mut().remove(request_id)
    }
}

impl std::fmt::Debug for Outgoing {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outgoing")
            .field("next_request_id", &self.next_request_id)
            .field("response_handlers", &"<response handlers>")
            .finish()
    }
}
