//
// axum_soketto main.rs
//
// Show how to mix compressed websockets with Axum by handling incoming
// requests at the hyper level.
//
// I've minimized the use of 'use' so it becomes explicit where all the
// various parts come from, as we're mixing a number of crates here and it
// is easy to get lost.
//
// There are two modules in this file: `ws` and `web`, the first uses
// soketto to handle websockets, and the second uses Axum to handle routing
// and provide ergonomics.
//
//
// Copyright 2024, Kees Verruijt
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//         http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    env_logger::init();

    let addr: std::net::SocketAddr = ([0, 0, 0, 0], 3000).into();
    let listener = tokio::net::TcpListener::bind(addr).await?;

    log::info!(
        "Listening on {:?} — connect via websocket or http...",
        listener.local_addr().unwrap()
    );

    let app = web::new();

    loop {
        let stream = match listener.accept().await {
            Ok((stream, addr)) => {
                log::info!("Accepting new connection: {addr}");
                stream
            }
            Err(e) => {
                log::error!("Accepting new connection failed: {e}");
                continue;
            }
        };

        let tower_service = app.clone();

        tokio::spawn(async {
            // Hyper has its own `AsyncRead` and `AsyncWrite` traits and doesn't use tokio.
            // `TokioIo` converts between them.
            let stream = hyper_util::rt::TokioIo::new(stream);

            // Hyper also has its own `Service` trait and doesn't use tower. We can use
            // `hyper::service::service_fn` to create a hyper `Service` that calls our app through
            // `tower::Service::call`.
            let hyper_service = hyper::service::service_fn(move |request| {
                handler(request, tower_service.clone())
            });

            // `server::conn::auto::Builder` supports both http1 and http2.
            //
            // `TokioExecutor` tells hyper to use `tokio::spawn` to spawn tasks.
            if
                let Err(err) = hyper_util::server::conn::auto::Builder
                    ::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection_with_upgrades(stream, hyper_service).await
            {
                log::error!("HTTP connection failed {err}");
            }
        });
    }
}

/// Handle incoming HTTP Requests.
///
/// Since a HTTP request always needs a response, the error case is Infallible.
/// We still need a Result<> wrapper because the async handling is handled by
/// returning errors under the covers.
///
async fn handler(
    req: hyper::Request<hyper::body::Incoming>,
    tower_service: axum::Router
) -> Result<axum::response::Response, std::convert::Infallible> {
    use tower::Service; // Import the trait that allows .call() on a service

    log::info!("Request: {:?}", req);

    if soketto::handshake::http::is_upgrade_request(&req) {
        ws::handle_upgrade_request(req)
    } else {
        // We have to clone `tower_service` because hyper's `Service` uses `&self` whereas
        // tower's `Service` requires `&mut self`.
        //
        // We don't need to call `poll_ready` since `Router` is always ready.
        tower_service.clone().call(req).await
    }
}

///
/// Return a response appropriate for a handler.
///
///
fn infallible_response(
    status: u16,
    message: String
) -> Result<axum::response::Response, std::convert::Infallible> {
    let body: axum::body::Body = message.into();
    let response: axum::response::Response<axum::body::Body> = axum::response::Response
        ::builder()
        .status(status)
        .body(body)
        .unwrap();
    Ok(response)
}

mod ws {
    use crate::infallible_response;

    pub fn handle_upgrade_request(
        req: hyper::Request<hyper::body::Incoming>
    ) -> Result<hyper::Response<axum::body::Body>, std::convert::Infallible> {
        //
        // You can route here if needed, maybe using something like pathrouter?
        //
        if !req.uri().path().ends_with("/path") {
            log::error!("Path does not match '/path'");
            return infallible_response(404, format!("No such path {}", req.uri().path()));
        }

        // Create a new handshake server.
        let mut server = soketto::handshake::http::Server::new();

        // Add the deflate extensions that we want to use.
        let mut deflate = soketto::extension::deflate::Deflate::new(soketto::Mode::Server);
        let query = req.uri().query().unwrap_or("");
        let enabled = query.contains("context_takeover");
        log::info!("{enabled} context takeover for client and server");
        deflate.set_client_context_takeover(enabled);
        deflate.set_server_context_takeover(enabled);

        server.add_extension(Box::new(deflate));

        // Attempt the handshake.
        match server.receive_request(&req) {
            // The handshake has been successful so far; return the response we're given back
            // and spawn a task to handle the long-running WebSocket server:
            Ok(response) => {
                tokio::spawn(async move {
                    if let Err(e) = websocket_echo_messages(server, req).await {
                        log::error!("Error upgrading to websocket connection: {}", e);
                    }
                });
                // We've upgraded the socket and it's owned by the spawned task now;
                Ok(response.map(|()| axum::body::Body::empty()))
            }
            // We tried to upgrade and failed early on; tell the client about the failure however we like:
            Err(e) => {
                log::error!("Could not upgrade connection: {}", e);
                infallible_response(500, format!("Upgrade error: {e}"))
            }
        }
    }

    /// Echo any messages we get from the client back to them
    async fn websocket_echo_messages(
        server: soketto::handshake::http::Server,
        req: hyper::Request<hyper::body::Incoming>
    ) -> Result<(), soketto::BoxedError> {
        use tokio_util::compat::TokioAsyncReadCompatExt;

        // The negotiation to upgrade to a WebSocket connection has been successful so far. Next, we get back the underlying
        // stream using `hyper::upgrade::on`, and hand this to a Soketto server to use to handle the WebSocket communication
        // on this socket.
        //
        // Note: awaiting this won't succeed until the handshake response has been returned to the client, so this must be
        // spawned on a separate task so as not to block that response being handed back.
        let stream = hyper::upgrade::on(req).await?;
        let io = hyper_util::rt::TokioIo::new(stream);
        let stream = futures::io::BufReader::new(futures::io::BufWriter::new(io.compat()));

        // Get back a reader and writer that we can use to send and receive websocket messages.
        let (mut sender, mut receiver) = server.into_builder(stream).finish();

        // Echo any received messages back to the client:
        let mut message = Vec::new();
        loop {
            message.clear();
            match receiver.receive_data(&mut message).await {
                Ok(soketto::Data::Binary(n)) => {
                    assert_eq!(n, message.len());
                    sender.send_binary_mut(&mut message).await?;
                    sender.flush().await?;
                }
                Ok(soketto::Data::Text(n)) => {
                    assert_eq!(n, message.len());
                    if let Ok(txt) = std::str::from_utf8(&message) {
                        let hello = format!("Hello, {txt}!");
                        sender.send_text(hello).await?;
                        sender.flush().await?;
                    } else {
                        break;
                    }
                }
                Err(soketto::connection::Error::Closed) => {
                    break;
                }
                Err(e) => {
                    eprintln!("Websocket connection error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }
}

mod web {
    use axum::{ response::Html, routing::get, Router };

    pub fn new() -> axum::Router {
        Router::new().route("/", get(root))
    }

    async fn root() -> Html<&'static str> {
        Html(
            r##"
        <html>
            <head>
                <script>
                var socket;

                function start_websocket(params) {
                    if (!!params) {
                        socket = new WebSocket("ws://localhost:3000/path?context_takeover");
                    } else {
                        socket = new WebSocket("ws://localhost:3000/path");
                    }
                    socket.onmessage = function(msg) { document.getElementById('response').innerHTML = msg.data; };
                    socket.onclose = function(err) { document.getElementById('response').innerHTML = err; setTimeout(start_websocket(params), 2000); }
                }
                start_websocket(false);
                </script>
            </head>
            <body>
                <h1>Axum + compressed Websocket via Soketto example</h1>
                <p>
                    <label for="co">Enable context takeover</label>
                    <input id="co" type="checkbox" onchange="start_websocket(this.checked)" value="Enable context takeover">
                </p>
                <p>Please type your name:</p>
                <p><input type="text" onchange="socket.send(this.value)"></p>
                <p>The response from the websocket:</p>
                <p><div id="response" style="background-color: grey; color: lightblue;">... response ...</div></p>
            </body>
        </html>
        "##
        )
    }
}
