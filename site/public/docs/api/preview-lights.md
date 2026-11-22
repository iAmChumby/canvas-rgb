# Light preview

Method and path: GET /api/v1/preview/lights.

Area: Preview.

## Request

**GET /api/v1/preview/lights**

The opening message is JSON mapping each light index to its id, repeated when the lights change. Each later binary frame holds a 16-bit little-endian light index, a 16-bit little-endian LED count, then that many RGB triples.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl "ws://127.0.0.1:9430/api/v1/preview/lights?token=$TOKEN"
# Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.
```

```js
const token = process.env.CANVAS_RGB_TOKEN;
const socket = new WebSocket("ws://127.0.0.1:9430/api/v1/preview/lights?token=" + encodeURIComponent(token) + "");
socket.binaryType = "arraybuffer";
socket.onmessage = (event) => {
  console.log(new Uint8Array(event.data));
};
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `WebSocket (binary frames)`.

No field list is published for this body.

| Query | Type | What it holds |
| --- | --- | --- |
| `token` | `string` | The API token, in the query string because a browser cannot set a header on a WebSocket handshake. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/preview` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/group` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/lights` | none | WebSocket (binary frames) |

## Example

**GET**

The opening message is JSON mapping each light index to its id, repeated when the lights change. Each later binary frame holds a 16-bit little-endian light index, a 16-bit little-endian LED count, then that many RGB triples.

This route takes no request body.
