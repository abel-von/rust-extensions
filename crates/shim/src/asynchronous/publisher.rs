use std::os::unix::io::RawFd;
use tokio::task::spawn_blocking;
use async_trait::async_trait;
use containerd_shim_protos::protobuf::Message;
use containerd_shim_protos::shim::{empty, events};
use containerd_shim_protos::shim_async::{Client, Events, EventsClient};
use containerd_shim_protos::ttrpc;
use containerd_shim_protos::ttrpc::r#async::TtrpcContext;
use containerd_shim_protos::ttrpc::context::Context;
use crate::asynchronous::utils::asyncify;
use crate::error::Result;
use crate::util::{any, connect, timestamp};

/// Async Remote publisher connects to containerd's TTRPC endpoint to publish events from shim.
pub struct RemotePublisher {
    client: EventsClient,
}

impl RemotePublisher {
    /// Connect to containerd's TTRPC endpoint asynchronously.
    ///
    /// containerd uses `/run/containerd/containerd.sock.ttrpc` by default
    pub async fn new(address: impl AsRef<str>) -> Result<RemotePublisher> {
        let client = Self::connect(address).await?;

        Ok(RemotePublisher {
            client: EventsClient::new(client),
        })
    }

    async fn connect(address: impl AsRef<str>) -> Result<Client> {
        let addr = address.as_ref().to_string();
        let fd = asyncify(move || -> Result<RawFd> {
            let fd = connect(addr)?;
            Ok(fd)
        }).await?;

        // Client::new() takes ownership of the RawFd.
        Ok(Client::new(fd))
    }

    /// Publish a new event.
    ///
    /// Event object can be anything that Protobuf able serialize (e.g. implement `Message` trait).
    pub async fn publish(
        &self,
        ctx: Context,
        topic: &str,
        namespace: &str,
        event: impl Message,
    ) -> Result<()> {
        let mut envelope = events::Envelope::new();
        envelope.set_topic(topic.to_owned());
        envelope.set_namespace(namespace.to_owned());
        envelope.set_timestamp(timestamp()?);
        envelope.set_event(any(event)?);

        let mut req = events::ForwardRequest::new();
        req.set_envelope(envelope);

        self.client.forward(ctx, &req).await?;

        Ok(())
    }
}

#[async_trait]
impl Events for RemotePublisher {
    async fn forward(
        &self,
        _ctx: &TtrpcContext,
        req: events::ForwardRequest,
    ) -> ttrpc::Result<empty::Empty> {
        self.client.forward(Context::default(), &req).await
    }
}