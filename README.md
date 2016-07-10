# Share-login

Tool to share accounts between multiple minetest servers.

It is separated in two parts:

1. A minetest mod written in lua, that provides an auth handler and talks with an HTTP server
2. A HTTP server written in rust, that takes care of the database

## Setup

### Server
As the first step, you'll have to set up the server part. For that, you'll need rust.

Any stable rust version starting with Rust 1.8.0 should work.
You can grab recent rust releases via [rustup.rs](https://www.rustup.rs/) should your package manager not offer Rust 1.8.0.
Install it like the website says, then do `rustup install stable`, and you should be all set.

Then, compile the server with `cargo build --release`.

You can then start the server with `cargo run --release`, or directly start the binary located at `target/release/share_login`.
The cargo variant has the additional feature that it automatically recompiles if there has been a change made to the source.

For the initial setup, start the server once, and close it again with ctrl+c. This will create a sqlite file.

Then, open the sqlite file (e.g. with the awesome [sqlitebrowser](http://sqlitebrowser.org/) tool), and fill the `servers` table with the servers you want to give access to.

The `id` column will be created automatically, and the `name` column is your free choice, but it needs to be unique. The `api_key` column should consist of a really long securely randomly generated sequence. Really, its a security relevant key, it should be 64 chars minimum.

All you need to give the server owners you want to give access to is the url of the server, and the `api_key` for their servers.

### Client

As the second step, you'll need to set up the client part (the lua mod). For this, add this
source directory as a mod to your server. You can e.g. use symlinks for this,
but you can also do a separate clone. It depends on what's more convenient for you. No access to any shared files is required.

Then, you'll need to change/add some settings:

1. Add the `share_login` to the `secure.http_mods` setting. If that setting is empty or non-existent (most likely the case), you can simply set it to the value `share_login`.
2. Set the `share_login_http_api_url` setting to the url where the sharing server will run
3. Set the `share_login_http_api_secret` setting to the server's individual api key.

## License

Licensed under the MIT license. For details, see the [LICENSE](LICENSE) file.
