extern crate rotor;
extern crate rotor_http;
extern crate rusqlite;
extern crate rustc_serialize;
#[macro_use]
extern crate log;
extern crate env_logger;

use std::time::Duration;

use rotor::{Scope, Time};
use rotor_http::server::{RecvMode, Server, Head, Response, Fsm};
use rotor::mio::tcp::TcpListener;

use rusqlite::{Connection, MappedRows, Statement, Error as RusqliteError};

use rustc_serialize::json;

trait VersionAccess {
	fn version(&self) -> Result<i32, RusqliteError>;
	fn set_version(&self, v :i32) -> Result<(), RusqliteError>;
}

impl VersionAccess for Connection {
	fn version(&self) -> Result<i32, RusqliteError> {
		Ok(try!(self.query_row("PRAGMA user_version", &[], |row| {
			row.get(0)
		})))
	}
	fn set_version(&self, v :i32) -> Result<(), RusqliteError> {
		// Preparing the statement does not work.
		// It needs to be string concatenation :(
		let stmt = format!("PRAGMA user_version = {}", v);
		try!(self.execute(&stmt, &[]));
		Ok(())
	}
}

struct PreparedStatements<'conn> {
	get_server_id :Statement<'conn>,
	get_auth :Statement<'conn>,
	create_auth_pl :Statement<'conn>,
	create_auth_pr :Statement<'conn>,
	set_password :Statement<'conn>,
	set_privileges :Statement<'conn>,
	record_login :Statement<'conn>,
}

type Context<'conn> = PreparedStatements<'conn>;

impl<'a> PreparedStatements<'a> {
	fn new<'conn>(conn :&'conn Connection) ->
			Result<PreparedStatements<'conn>, RusqliteError> {
		Ok(PreparedStatements {
			get_server_id : try!(conn.prepare("SELECT id FROM servers WHERE api_key = ?")),
			get_auth : try!(conn.prepare("
				SELECT password, pw_override, last_login, privs
				FROM players
				LEFT JOIN players_on_servers
					ON players.id = players_on_servers.player_id
				WHERE players.name = ?
					AND players_on_servers.server_id = ?")),
			create_auth_pl : try!(conn.prepare("
				INSERT INTO players (name, password, last_login)
				VALUES (?, ?, ?)")),
			create_auth_pr : try!(conn.prepare("
				INSERT INTO players_on_servers(server_id, player_id, privs, pw_override)
				VALUES (?, (SELECT id FROM players WHERE name = ?), ?, ?)")),
			set_password : try!(conn.prepare("
				UPDATE players SET password = ? WHERE name = ?")),
			set_privileges : try!(conn.prepare("
				UPDATE players_on_servers
				SET privs = ?
				WHERE server_id = ?
					AND player_id = (SELECT id from players WHERE name = ?)")),
			record_login : try!(conn.prepare("
				UPDATE players
				SET last_login = ?
				WHERE name = ?")),
		})
	}
}

#[derive(Debug)]
enum Request<'a> {
	ValidCommand(String, Command),
	PageNotFound,
	// ugly ugly hack bc the compiler gives us E0207 on the impl below,
	// if we didnt make Request dependent on something
	#[allow(dead_code)] // disable the warning
	Phantom(std::marker::PhantomData<&'a ()>),
}

#[derive(Debug, Clone)]
enum Command {
	GetAuth,
	CreateAuth,
	SetPassword,
	SetPrivileges,
	RecordLogin,
}

fn send_string(res :&mut Response, data :&[u8]) {
	res.status(200, "OK");
	res.add_length(data.len() as u64).unwrap();
	res.done_headers().unwrap();
	res.write_body(data);
	res.done();
}

fn send_error(res :&mut Response, code :u16, msg :&str) {
	let data :&[u8] = msg.as_ref();
	res.status(code, msg);
	res.add_length(data.len() as u64).unwrap();
	res.done_headers().unwrap();
	res.write_body(data);
	res.done();
}

fn optionalize<'a, T,  F: FnMut(&rusqlite::Row)->T>(rows :Result<MappedRows<'a,F>, RusqliteError>)  ->
		Result<Option<T>, RusqliteError> {
	let mut res :Option<T> = None;
	for row in try!(rows) {
		res = Some(try!(row));
		break;
	}
	return Ok(res);
}

impl<'a> Server for Request<'a> {
	type Seed = ();
	type Context = PreparedStatements<'a>;
	fn headers_received(_seed :(), head :Head, _res :&mut Response,
		scope: &mut Scope<Context>)
		-> Option<(Self, RecvMode, Time)>
	{
		use self::Request::*;
		use self::Command::*;
		let mut api_secret = None;
		for header in head.headers {
			if header.name == "X-Minetest-ApiSecret" {
				api_secret = Some(String::from(
					std::str::from_utf8(header.value).unwrap_or("invalid-utf8")
				));
				break;
			}
		}
		let ret = match api_secret {
			Some(s) => match head.path {
				"/v1/get_auth" => ValidCommand(s, GetAuth),
				"/v1/create_auth" => ValidCommand(s, CreateAuth),
				"/v1/set_password" => ValidCommand(s, SetPassword),
				"/v1/set_privileges" => ValidCommand(s, SetPrivileges),
				"/v1/record_login" => ValidCommand(s, RecordLogin),
				_ => PageNotFound,
			},
			None => PageNotFound
		};

		info!{"{} => {:?}", head.path, ret};
		Some((ret, RecvMode::Buffered(1024), scope.now() + Duration::new(10, 0)))
	}
	fn request_received(self, body :&[u8], res :&mut Response,
		scope: &mut Scope<Context>)
		-> Option<Self>
	{
		use self::Request::*;
		use self::Command::*;
		match self {
		ValidCommand(api_secret, command) => {
			macro_rules! ttry {
			($expr:expr) => (match $expr {
				$crate::std::result::Result::Ok(val) => val,
				$crate::std::result::Result::Err(err) => {
					info!{"Internal server error: {}", err}
					send_error(res, 500, "Internal Server error");
					return None;
				}
			})
			}

			info!{"API SECRET => {:?}", api_secret};
			let server_id :Option<i64> = ttry!(optionalize(
				scope.get_server_id.query_map(&[&api_secret], |row| row.get(0))
			));

			info!{"SERVER ID => {:?}", server_id};
			match server_id {
			Some(ref srv_id) => {
				match command {
				GetAuth => {
					#[derive(RustcDecodable)]
					struct GetAuthData {
						name :String,
					}
					let d :GetAuthData = ttry!(json::decode(
						ttry!(std::str::from_utf8(body))
					));
					let rows = scope.get_auth.query_map(&[&d.name, srv_id], |row|
						(row.get(0), row.get(1), row.get(2), row.get(3)));
					let pw_llogin_privs :Option<(String, String, String, String)> =
						ttry!(optionalize(rows));

					#[derive(RustcEncodable)]
					struct AuthAnswer {
						password :String,
						privileges :String,
						last_login :String,
					}
					match pw_llogin_privs {
					Some((pw, pw_override, privs, llogin)) => {
						let ans = AuthAnswer {
							password : if pw_override == "" { pw } else { pw_override },
							privileges : privs,
							last_login : llogin,
						};
						let encoded = ttry!(json::encode(&ans));
						send_string(res, encoded.as_str().as_ref());
					},
					None => send_error(res, 404, ""),
					}
				},
				CreateAuth => {
					#[derive(RustcDecodable)]
					struct CreateAuthData {
						name :String,
						password :String,
						privileges :String,
					}
					let d :CreateAuthData = ttry!(json::decode(
						ttry!(std::str::from_utf8(body))
					));
					ttry!(scope.create_auth_pl.execute(&[
						&d.name, &d.password, &""]));
					ttry!(scope.create_auth_pr.execute(&[
						srv_id, &d.name, &d.privileges, &::rusqlite::types::Null]));
					send_string(res, &[]);
				},
				SetPassword => {
					#[derive(RustcDecodable)]
					struct SetPasswordData {
						name :String,
						password :String,
					}
					let d :SetPasswordData = ttry!(json::decode(
						ttry!(std::str::from_utf8(body))
					));
					ttry!(scope.set_password.execute(&[
						&d.name, &d.password]));
					send_string(res, &[]);
				},
				SetPrivileges => {
					#[derive(RustcDecodable)]
					struct SetPrivsData {
						name :String,
						privileges :String,
					}
					let d :SetPrivsData = ttry!(json::decode(
						ttry!(std::str::from_utf8(body))
					));
					ttry!(scope.set_privileges.execute(&[
						&d.privileges, srv_id, &d.name]));
					send_string(res, &[]);
				},
				RecordLogin => {
					#[derive(RustcDecodable)]
					struct RecordLoginData {
						name :String,
						last_login :f64,
					}
					let d :RecordLoginData = ttry!(json::decode(
						ttry!(std::str::from_utf8(body))
					));
					ttry!(scope.record_login.execute(&[
						&d.last_login, &d.name]));
					send_string(res, &[]);
				},
				}
			},
			None => {
				send_error(res, 401, "Unauthorized");
			},
			}
		},
		PageNotFound => {
			send_error(res, 404, "Not found");
		}
		Phantom(_) => unreachable!(),
		}
		None
	}
	fn request_chunk(self, _chunk: &[u8], _response: &mut Response,
		_scope: &mut Scope<Context>)
		-> Option<Self>
	{
		unreachable!();
	}

	/// End of request body, only for Progressive requests
	fn request_end(self, _response: &mut Response, _scope: &mut Scope<Context>)
		-> Option<Self>
	{
		unreachable!();
	}

	fn timeout(self, _response: &mut Response, _scope: &mut Scope<Context>)
		-> Option<(Self, Time)>
	{
		unimplemented!();
	}
	fn wakeup(self, _response: &mut Response, _scope: &mut Scope<Context>)
		-> Option<Self>
	{
		unimplemented!();
	}
}

fn create_db(conn :&Connection) {
	conn.execute("CREATE TABLE servers (
		id              INTEGER PRIMARY KEY,
		name            TEXT UNIQUE NOT NULL,
		api_key         TEXT UNIQUE NOT NULL
		)", &[]).unwrap();
	conn.execute("CREATE TABLE players (
		id              INTEGER PRIMARY KEY,
		name            TEXT UNIQUE NOT NULL,
		password        TEXT NOT NULL,
		last_login      TEXT NOT NULL
		)", &[]).unwrap();
	conn.execute("CREATE TABLE players_on_servers (
		server_id       INTEGER NOT NULL,
		player_id       INTEGER NOT NULL,
		privs           TEXT NOT NULL,
		pw_override     TEXT,
		PRIMARY KEY (server_id, player_id),
		FOREIGN KEY (server_id) REFERENCES servers(id),
		FOREIGN KEY (player_id) REFERENCES players(id)
		)", &[]).unwrap();
	conn.execute("CREATE INDEX servers_api_key_idx ON servers (api_key)", &[]).unwrap();
	conn.execute("CREATE INDEX players_name_idx ON players (name)", &[]).unwrap();

	conn.set_version(1).unwrap();
}

fn main() {
	env_logger::init().unwrap();
	let listen_addr = "127.0.0.1:8000";
	println!("Starting http server on http://{}/", listen_addr);

	let db_path = "database.sqlite";
	let conn = Connection::open(db_path).unwrap();
	let cur_version = conn.version().unwrap();
	if cur_version == 0 {
		create_db(&conn);
	} else {
		assert_eq!(cur_version, 1); // only supported version is 1
	}

	let event_loop = rotor::Loop::new(&rotor::Config::new()).unwrap();
	let mut loop_inst = event_loop.instantiate(PreparedStatements::new(&conn).unwrap());

	let lst = TcpListener::bind(&{let ar :&str = listen_addr.as_ref(); ar }
		.parse().unwrap()).unwrap();

	loop_inst.add_machine_with(|scope| {
		Fsm::<Request, _>::new(lst, (), scope)
	}).unwrap();
	loop_inst.run().unwrap();
}
