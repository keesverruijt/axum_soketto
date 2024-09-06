### Axum + Soketto example

Axum's websocket support unfortunately doesn't support compression yet.
This shows a workaround: use Soketto over hyper as the WebSockets layer.

This is a mix of Axum's [serve-with-hyper](https://github.com/tokio-rs/axum/blob/main/examples/serve-with-hyper/src/main.rs)
and Soketto's [hyper-server](https://github.com/paritytech/soketto/blob/master/examples/hyper_server.rs) examples.

# Demo

run this demo with:

    RUST_LOG=debug cargo run

Then, on the same machine, open a browser and go to http://localhost:3000 .

In the output you should see the HTML request and one WebSocket request, with this message:

    [2024-09-05T08:28:46Z DEBUG soketto::connection] xxxxxxx: using extension: permessage-deflate


