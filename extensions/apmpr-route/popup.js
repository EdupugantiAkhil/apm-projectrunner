const routesElement = document.querySelector("#routes");
const statusElement = document.querySelector("#connection-status");
const cableElement = document.querySelector("#signal-cable");
const disconnectButton = document.querySelector("#disconnect");
const errorElement = document.querySelector("#error");
let tabId;

function showError(message) {
  errorElement.textContent = message;
  errorElement.hidden = false;
}

function renderState(active) {
  document.querySelectorAll(".route").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.route === active));
  });
  const connected = Boolean(active);
  statusElement.textContent = connected ? `Connected / ${active}` : "Not connected";
  statusElement.classList.toggle("connected", connected);
  cableElement.classList.toggle("connected", connected);
  disconnectButton.hidden = !connected;
}

function request(message) {
  return chrome.runtime.sendMessage({ ...message, tabId }).then((response) => {
    if (!response?.ok) {
      throw new Error(response?.error || "The route could not be changed.");
    }
    return response.result;
  });
}

function routeButton(route) {
  const button = document.createElement("button");
  button.className = "route";
  button.type = "button";
  button.dataset.route = route.id;
  button.setAttribute("aria-pressed", "false");
  button.textContent = route.label;
  button.addEventListener("click", async () => {
    errorElement.hidden = true;
    try {
      renderState((await request({ operation: "connect", route: route.id })).active);
    } catch (error) {
      showError(error.message);
    }
  });
  return button;
}

disconnectButton.addEventListener("click", async () => {
  errorElement.hidden = true;
  try {
    renderState((await request({ operation: "disconnect" })).active);
  } catch (error) {
    showError(error.message);
  }
});

async function initialize() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) {
    throw new Error("Open a normal browser tab before choosing a route.");
  }
  tabId = tab.id;
  document.querySelector("#tab-title").textContent = tab.title || tab.url || `Tab ${tabId}`;
  const state = await request({ operation: "state" });
  if (state.routes.length === 0) {
    routesElement.textContent = "No routes are declared in routes.js.";
    return;
  }
  state.routes.forEach((route) => routesElement.append(routeButton(route)));
  renderState(state.active);
}

initialize().catch((error) => showError(error.message));
