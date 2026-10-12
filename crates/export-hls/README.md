# export-hls

Zero-dependency cleartext HLS media playlist parser and HTTP/1.1 fetcher.

This crate reads cleartext playlists only. It does not download YouTube and it does not transcode.

`#EXT-X-KEY`, `https` URLs, cookies, credentials, and userinfo are unsupported. Loopback is refused unless the caller allows it. Link-local is always refused, including after a redirect. The socket fetcher is not built for `wasm32-unknown-unknown`; the parser is.
