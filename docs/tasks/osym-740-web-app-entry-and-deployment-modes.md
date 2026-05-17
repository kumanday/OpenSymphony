---
id: OSYM-740
title: Web App Entry And Deployment Modes
milestone: "M10: Web Client And External Gateway"
priority: 2
estimate: 5
blockedBy: ["OSYM-710"]
blocks: ["OSYM-741", "OSYM-742"]
parent: null
---

## Summary

Create the browser app entrypoint and deployment modes for gateway-served and separately deployed web clients.

## Scope

### In scope

- Add browser app wrapper.
- Configure environment-based gateway URL.
- Configure static asset build.
- Ensure Tauri APIs do not leak into the browser bundle.
- Serve static assets from the OpenSymphony Gateway.
- Support base path deployment, cache-busted assets, and local development proxy.
- Document separate static deployment with gateway base URL configuration.

### Out of scope

- Hosted login implementation.
- Browser transport reconnect internals.
- Hosted infrastructure deployment.

## Deliverables

- Web app build.
- Gateway-served static asset path.
- Separately deployed web configuration.
- Browser smoke tests.
- Deployment docs.

## Acceptance Criteria

- [ ] The web app builds without desktop-only dependencies.
- [ ] The gateway can serve the built web app for local/external mode.
- [ ] The web app can be configured to point at a separate gateway URL.

## Test Plan

- Run browser build and smoke tests.
- Verify static asset serving through the gateway in local development.
- Verify standalone deployment configuration with a test gateway URL.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.3 and release 3, `docs/host-client-implementation_plan.md` P7.1/P7.3/P7.4.
- The browser app shares the dashboard, task graph, run, planning, and approval components with desktop.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use the same shared frontend packages created for desktop.
