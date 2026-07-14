importScripts("routes.js");

const HEADER = "X-Switchyard-Route";
const ROUTE_ID = /^[a-z0-9][a-z0-9._:-]{0,127}$/;
const RESOURCE_TYPES = [
  "main_frame",
  "sub_frame",
  "xmlhttprequest",
  "websocket",
  "other",
  "ping"
];

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizeEndpoint(value) {
  const endpoint = new URL(value);
  if (endpoint.protocol !== "http:" && endpoint.protocol !== "https:") {
    throw new Error(`Endpoint ${value} must use HTTP or HTTPS.`);
  }
  const local = endpoint.hostname === "localhost"
    || endpoint.hostname === "127.0.0.1"
    || endpoint.hostname === "[::1]"
    || endpoint.hostname.endsWith(".localhost");
  if (!local || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
    throw new Error(`Endpoint ${value} is not an allowed local origin or path.`);
  }
  return endpoint.href;
}

function configuredRoutes() {
  const identifiers = new Set();
  return globalThis.SWITCHYARD_ROUTES.map((route) => {
    if (!ROUTE_ID.test(route.id) || identifiers.has(route.id)) {
      throw new Error(`Route id ${route.id} is invalid or duplicated.`);
    }
    identifiers.add(route.id);
    if (typeof route.label !== "string" || route.label.trim() === "") {
      throw new Error(`Route ${route.id} needs a label.`);
    }
    if (!Array.isArray(route.endpoints) || route.endpoints.length === 0) {
      throw new Error(`Route ${route.id} needs at least one endpoint.`);
    }
    return {
      id: route.id,
      label: route.label,
      endpoints: route.endpoints.map(normalizeEndpoint)
    };
  });
}

function ruleFor(tabId, route) {
  const destinations = route.endpoints
    .map((endpoint) => `${escapeRegex(endpoint)}.*`)
    .join("|");
  return {
    id: tabId,
    priority: 1,
    action: {
      type: "modifyHeaders",
      requestHeaders: [{
        header: HEADER,
        operation: "set",
        value: route.id
      }]
    },
    condition: {
      tabIds: [tabId],
      regexFilter: `^(?:${destinations})$`,
      resourceTypes: RESOURCE_TYPES
    }
  };
}

async function activeRoutes() {
  return (await chrome.storage.session.get("tabs")).tabs || {};
}

async function connect(tabId, routeId) {
  const route = configuredRoutes().find((candidate) => candidate.id === routeId);
  if (!route) {
    throw new Error(`Route ${routeId} is not declared.`);
  }
  await chrome.declarativeNetRequest.updateSessionRules({
    removeRuleIds: [tabId],
    addRules: [ruleFor(tabId, route)]
  });
  const tabs = await activeRoutes();
  tabs[tabId] = route.id;
  await chrome.storage.session.set({ tabs });
  return route;
}

async function disconnect(tabId) {
  await chrome.declarativeNetRequest.updateSessionRules({ removeRuleIds: [tabId] });
  const tabs = await activeRoutes();
  delete tabs[tabId];
  await chrome.storage.session.set({ tabs });
}

chrome.tabs.onRemoved.addListener((tabId) => {
  disconnect(tabId).catch(() => {});
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  const respond = async () => {
    const tabId = Number(message.tabId);
    if (!Number.isSafeInteger(tabId) || tabId <= 0) {
      throw new Error("The active tab is unavailable.");
    }
    if (message.operation === "state") {
      const tabs = await activeRoutes();
      return { routes: configuredRoutes(), active: tabs[tabId] || null };
    }
    if (message.operation === "connect") {
      const route = await connect(tabId, message.route);
      return { active: route.id };
    }
    if (message.operation === "disconnect") {
      await disconnect(tabId);
      return { active: null };
    }
    throw new Error("Unknown extension operation.");
  };
  respond()
    .then((result) => sendResponse({ ok: true, result }))
    .catch((error) => sendResponse({ ok: false, error: error.message }));
  return true;
});
