share_login = {}

local http_api = minetest.request_http_api()

if not http_api then
	local mod_name = minetest.get_current_modname()
	error("Can't obtain http api handle. Please add the "
		.. mod_name .. " mod to the secure.http_mods setting.")
end

local http_api_url = minetest.setting_get("share_login_http_api_url")
local http_api_secret = minetest.setting_get("share_login_http_api_secret")

if type(http_api_url) ~= "string" then
	error("Setting share_login_http_api_url setting not set!")
end

if type(http_api_secret) ~= "string" then
	error("Setting share_login_http_api_secret setting not set!")
end

-- busy waiting, its not really optimal :/
-- either way, during this time the server is
-- blocking this way or another, whether its busy or not
-- isn't really that important.
local clock = os.clock
local function sleep(n)
	local t0 = clock()
	while clock() - t0 <= n do end
end

local function wait_for_http_completion(handle)
	local tr = 0.001
	local last_res = http_api.fetch_async_get(handle)
	while not last_res.completed do
		sleep(tr)
		if tr < 0.010 then
			tr = tr +0.001
		end
		last_res = http_api.fetch_async_get(handle)
	end
	core.log('action', "Bla '" .. tr .. "'.")
	return last_res
end

local function execute_request(request)
	local handle = http_api.fetch_async(request)
	return wait_for_http_completion(handle)
end

local function generate_request(url_append)
	return {
		url = http_api_url .. url_append,
		extra_headers = { "X-Minetest-ApiSecret: " .. http_api_secret },
	}
end

share_login.auth_handler = {
	get_auth = function(name)
		assert(type(name) == "string")
		local req = generate_request("v1/get_auth")
		req.post_data = minetest.write_json({ name = name })
		local res = execute_request(req)
		-- If request not succeeded, return nil
		local function error_log(msg)
			core.log('error', "Error with processing auth request for '" ..
					name .. "': " .. msg)
		end
		if not res.succeeded then
			error_log("Could not reach auth server")
			-- TODO do something smarter here, like returning an entry whose password is always false
			return nil
		end
		local res_data = minetest.parse_json(res.data)
		if not res_data then
			-- If the player is not found, return nil
			return nil
		end

		-- This is only a very cursory check to sort out obvious mistakes
		-- A remote auth server still can crash our server
		if not (res_data.password and res_data.privileges and res_data.last_login) then
			error_log("Invalid data received from auth server. " ..
				"The data: '" .. res.data .. "'")
			-- TODO do something smarter here, like returning an entry whose password is always false
			return nil
		end
		res_data.privileges = minetest.string_to_privs(res_data.privileges)

		-- For the admin, give everything
		if name == core.setting_get("name") then
			for priv, def in pairs(core.registered_privileges) do
				res_data.privileges[priv] = true
			end
		end

		-- All done
		return res_data
	end,
	create_auth = function(name, password)
		assert(type(name) == "string")
		assert(type(password) == "string")
		core.log('info', "Authentication handler adding player '"..name.."'")
		local req = generate_request("v1/create_auth")
		req.post_data = minetest.write_json({
			name = name,
			password = password,
			privileges = core.setting_get("default_privs")
		})
		local res = execute_request(req)
		-- If request not succeeded, return nil
		local function error_log(msg)
			core.log('error', "Error with processing create_auth request for '" ..
					name .. "': " .. msg)
		end
		if not res.succeeded then
			error_log("Could not reach auth server")
		elseif not res.code == 200 then
			error_log("Response code is not 200, but " .. res.code)
		end
	end,
	set_password = function(name, password)
		assert(type(name) == "string")
		assert(type(password) == "string")
		core.log('info', "Authentication handler setting password of player '"..name.."'")
		local req = generate_request("v1/set_password")
		req.post_data = minetest.write_json({
			name = name,
			password = password,
		})
		local res = execute_request(req)
		-- If request not succeeded, return nil
		local function error_log(msg)
			core.log('error', "Error with processing create_auth request for '" ..
					name .. "': " .. msg)
		end
		if not res.succeeded then
			error_log("Could not reach auth server")
			return false
		elseif not res.code == 200 then
			error_log("Response code is not 200, but " .. res.code)
			return false
		end
		return true
	end,
	set_privileges = function(name, privileges)
		assert(type(name) == "string")
		assert(type(privileges) == "table")
		local req = generate_request("v1/set_privileges")
		req.post_data = minetest.write_json({
			name = name,
			privileges = minetest.privs_to_string(privileges),
		})
		local res = execute_request(req)
		-- If request not succeeded, return nil
		local function error_log(msg)
			core.log('error', "Error with processing set_privileges request for '" ..
					name .. "': " .. msg)
		end
		if not res.succeeded then
			error_log("Could not reach auth server")
			return false
		elseif not res.code == 200 then
			error_log("Response code is not 200, but " .. res.code)
			return false
		end
		minetest.notify_authentication_modified(name)
		return true
	end,
	reload = function()
		return true
	end,
	record_login = function(name)
		assert(type(name) == "string")
		local req = generate_request("v1/record_login")
		req.post_data = minetest.write_json({
			name = name,
			last_login = os.time(),
		})
		local res = execute_request(req)
		-- If request not succeeded, return nil
		local function error_log(msg)
			core.log('error', "Error with processing record_login request for '" ..
					name .. "': " .. msg)
		end
		if not res.succeeded then
			error_log("Could not reach auth server")
			return false
		elseif not res.code == 200 then
			error_log("Response code is not 200, but " .. res.code)
			return false
		end
		return true
	end,
}

-- Register the shared auth handler
if not minetest.is_singleplayer() then
	minetest.register_authentication_handler(share_login.auth_handler)
	core.log('action', "share_login: Registered auth handler")
else
	core.log('action', "share_login: Not adding auth handler because of singleplayer game")
end