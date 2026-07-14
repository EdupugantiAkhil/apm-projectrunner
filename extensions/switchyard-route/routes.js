// This file is the extension's trust boundary. Keep only routes and local endpoints
// declared by your Switchyard deployment, then reload the unpacked extension.
globalThis.SWITCHYARD_ROUTES = Object.freeze([
  {
    id: "ui-1",
    label: "UI 1 → backend 1",
    endpoints: ["http://localhost:10081/"]
  },
  {
    id: "ui-2",
    label: "UI 2 → backend 2",
    endpoints: ["http://localhost:10081/"]
  }
]);
