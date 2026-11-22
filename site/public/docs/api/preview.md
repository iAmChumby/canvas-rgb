# Preview

Method and path: GET /api/v1/preview.

Area: Preview.

## Request

**GET /api/v1/preview**

Streams the whole rendered picture. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples. With stack=true it streams the stack preview; without it, the running effect.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl "ws://127.0.0.1:9430/api/v1/preview?token=$TOKEN&stack=true"
# Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.
```

```js
const token = process.env.CANVAS_RGB_TOKEN;
const socket = new WebSocket("ws://127.0.0.1:9430/api/v1/preview?token=" + encodeURIComponent(token) + "&stack=true");
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
| `stack` | `boolean` | True streams the stack preview engine. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/preview` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/group` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/lights` | none | WebSocket (binary frames) |

## Example

**GET**

Streams the whole rendered picture. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples. With stack=true it streams the stack preview; without it, the running effect.

This route takes no request body.
