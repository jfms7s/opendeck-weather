// Runs the property inspector's <script> against a stub DOM and a fake
// WebSocket, and checks what it sends. No dependencies:
// `node --test tests/*.test.mjs`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const html = readFileSync(new URL("../assets/propertyInspector/index.html", import.meta.url), "utf8");
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
assert.equal(scripts.length, 1, "expected one inline <script>");
const source = scripts[0][1];

const FORECAST = "com.jfms7s.weather.forecast";
const LISBON = { name: "Lisbon", label: "Lisbon, Portugal", latitude: 38.72509, longitude: -9.1498 };
const PORTO = { name: "Porto", label: "Porto, Portugal", latitude: 41.14961, longitude: -8.61099 };

function element() {
	const classes = new Set();
	const listeners = {};
	return {
		value: "",
		textContent: "",
		innerHTML: "",
		children: [],
		dataset: {},
		classList: {
			toggle(name, force) {
				const on = force ?? !classes.has(name);
				if (on) classes.add(name);
				else classes.delete(name);
				return on;
			},
			contains: (name) => classes.has(name),
		},
		addEventListener(type, fn) {
			(listeners[type] ??= []).push(fn);
		},
		fire(type, event = {}) {
			for (const fn of listeners[type] ?? []) fn(event);
		},
		appendChild(child) {
			this.children.push(child);
		},
		get hidden() {
			return classes.has("hidden");
		},
	};
}

/** Loads the inspector for `settings` and opens its socket. */
async function inspector(settings, action = FORECAST) {
	const elements = {};
	const document = {
		getElementById: (id) => (elements[id] ??= element()),
		createElement: () => element(),
		querySelectorAll: () => [],
	};
	const sockets = [];
	class WebSocket {
		static OPEN = 1;
		constructor(url) {
			this.url = url;
			this.readyState = 0;
			this.sent = [];
			sockets.push(this);
		}
		send(data) {
			this.sent.push(JSON.parse(data));
		}
	}
	const context = vm.createContext({ document, WebSocket, console });
	context.window = context;
	vm.runInContext(source, context);
	context.connectOpenActionSocket(
		"1234",
		"ctx",
		"registerPropertyInspector",
		"{}",
		JSON.stringify({ action, payload: { settings } }),
	);
	await new Promise((resolve) => setImmediate(resolve));
	const socket = sockets[0];
	socket.readyState = WebSocket.OPEN;
	socket.onopen();
	const $ = (id) => document.getElementById(id);
	return {
		$,
		/** The plugin (or OpenDeck) sends the inspector a message. */
		receive: (payload) =>
			socket.onmessage({ data: JSON.stringify({ event: "sendToPropertyInspector", payload }) }),
		sent: () => socket.sent.splice(0),
		saved: () => socket.sent.filter((m) => m.event === "setSettings").map((m) => m.payload),
	};
}

test("opening registers, then asks for the default location", async () => {
	const pi = await inspector({});
	const [register, ask] = pi.sent();
	assert.deepEqual(register, { event: "registerPropertyInspector", uuid: "ctx" });
	assert.equal(ask.event, "sendToPlugin");
	assert.deepEqual(ask.payload, { event: "getDefaultLocation" });
});

test("a Units change saves the units and keeps every other field", async () => {
	const pi = await inspector({ location: LISBON, day: 3, aqi_scale: "european" });
	pi.sent();
	pi.$("units").value = "imperial";
	pi.$("units").fire("change");
	assert.deepEqual(pi.saved(), [{ location: LISBON, day: 3, aqi_scale: "european", units: "imperial" }]);
});

test("picking a search result saves name, label and coordinates", async () => {
	const pi = await inspector({ units: "metric" });
	pi.sent();
	pi.$("query").value = "Porto";
	pi.$("search").fire("click");
	const search = pi.sent().find((m) => m.event === "sendToPlugin");
	assert.deepEqual(search.payload, { event: "search", query: "Porto" });

	pi.receive({ event: "searchResults", query: "Porto", results: [LISBON, { ...PORTO, admin: "x" }] });
	assert.equal(pi.$("results").hidden, false);
	pi.$("results").value = "1";
	pi.$("results").fire("change");
	assert.deepEqual(pi.saved(), [{ units: "metric", location: PORTO }]);
	assert.equal(pi.$("current").textContent, "Porto, Portugal");
});

test("results for an older query are ignored", async () => {
	const pi = await inspector({});
	pi.$("query").value = "Porto";
	pi.receive({ event: "searchResults", query: "Port", results: [PORTO] });
	assert.equal(pi.$("results").children.length, 0);
	assert.notEqual(pi.$("status").textContent, "Pick a match:");
});

test("Use the default instead drops the own location", async () => {
	const pi = await inspector({ location: PORTO, units: "imperial" });
	pi.receive({ event: "defaultLocation", location: LISBON });
	assert.equal(pi.$("follow-default").hidden, false);
	pi.$("follow-default").fire("click");
	assert.deepEqual(pi.saved(), [{ units: "imperial" }]);
	assert.equal(pi.$("current").textContent, "Lisbon, Portugal (default)");
});

test("Use for all actions is offered only when the own location is not already the default", async () => {
	const pi = await inspector({ location: { ...LISBON } });
	assert.equal(pi.$("make-default").hidden, false, "no default yet");
	pi.receive({ event: "defaultLocation", location: LISBON });
	assert.equal(pi.$("make-default").hidden, true, "already the default");
	pi.receive({ event: "defaultLocation", location: PORTO });
	assert.equal(pi.$("make-default").hidden, false);
	pi.sent();
	pi.$("make-default").fire("click");
	const [message] = pi.sent();
	assert.deepEqual(message.payload, { event: "setDefaultLocation", location: LISBON });
});
