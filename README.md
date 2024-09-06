### Axum + Soketto example

Axum's websocket support unfortunately doesn't support compression yet.
This shows a workaround: use Soketto over hyper as the WebSockets layer.

This is a mix of Axum's [serve-with-hyper](https://github.com/tokio-rs/axum/blob/main/examples/serve-with-hyper/src/main.rs)
and Soketto's [hyper-server](https://github.com/paritytech/soketto/blob/master/examples/hyper_server.rs) examples.

# Demo

Run this demo with:

    RUST_LOG=trace cargo run

Then, on the same machine, open a browser and go to http://localhost:3000 .

You can now send websocket requests, and a reply saying 'Hello <your data>' is sent back.

Click the 'Enable context takeover' button to allow context to be kept between messages, this will increase the
compression ratio as the compressed data can refer back to previous message content.


