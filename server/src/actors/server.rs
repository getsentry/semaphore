use ::actix::prelude::*;
use actix_web::server::StopServer;
use futures::prelude::*;

use semaphore_common::{metric, Config};

use crate::actors::controller::{Controller, Shutdown, Subscribe, TimeoutError};
use crate::service::{self, ServiceState};

pub use crate::service::ServerError;

pub struct Server {
    http_server: Recipient<StopServer>,
}

impl Server {
    pub fn start(config: Config) -> Result<Addr<Self>, ServerError> {
        metric!(counter("server.starting") += 1);
        // spawn our own services into the default arbiter
        let service_state = ServiceState::start(config)?;

        // spawn the HTTP server into a separate arbiter to make connects faster
        Ok(Arbiter::start(|_ctx| {
            let http_server = service::start(service_state).unwrap();
            Server { http_server }
        }))
    }
}

impl Actor for Server {
    type Context = Context<Self>;

    fn started(&mut self, context: &mut Self::Context) {
        Controller::from_registry().do_send(Subscribe(context.address().recipient()));
    }
}

impl Handler<Shutdown> for Server {
    type Result = ResponseFuture<(), TimeoutError>;

    fn handle(&mut self, message: Shutdown, _context: &mut Self::Context) -> Self::Result {
        let graceful = message.timeout.is_some();

        // We assume graceful shutdown if we're given a timeout. The actix-web http server is
        // configured with the same timeout, so it will match. Unfortunately, we have to drop any
        // errors  and replace them with the generic `TimeoutError`.
        let future = self
            .http_server
            .send(StopServer { graceful })
            .map_err(|_| TimeoutError)
            .and_then(|result| result.map_err(|_| TimeoutError));

        Box::new(future)
    }
}
