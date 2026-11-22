---
title: Developer docs
summary: The local API, the command line client and the effect file format.
order: 1
---

# Developer docs

canvas-rgb runs on Windows. A daemon drives the lights and the fans and serves a local HTTP API. A separate fan and update service runs as SYSTEM. The tray and the CLI talk to the daemon's API.

The API listens on 127.0.0.1:9430, with every route under /api/v1. A bearer token is checked on each request. Status needs no token; the three preview sockets take it as `?token=` in the URL because a browser cannot set a header on a WebSocket handshake. The Host and Origin headers are checked as well.

The CLI drives the same API from a terminal. It checks who owns port 9430 before it reads the token file or sends a request. `canvas-rgb status` reports what the daemon is doing.

An effect is one HTML file. The runtime reads the head for the name and properties, runs the scripts on QuickJS and draws with a Canvas 2D context. SignalRGB effects use the same format and run as they are when they use the supported calls.

## Start from a task

The getting-started page covers the token and the guards. The reference lists all 66 routes with their request and answer shapes, checked against the route definitions. The CLI page lists the commands and flags. The effects page shows how to write and check a file.

## Before the first request

The token is a bearer secret. Check that port 9430 belongs to the daemon before the token is sent; the getting-started page shows the check for both PowerShell and a shell.
