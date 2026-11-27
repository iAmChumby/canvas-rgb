# Stack preview

Method and path: PUT / DELETE /api/v1/stacks/preview.

Area: Stacks and patterns.

## Request

**PUT /api/v1/stacks/preview**

Starts or updates the stack used by the preview engine, for a group id the active scene has.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"stack":{"id":"stack:example","layers":[]},"epoch":"a1b2c3d4e5f60718","seq":1}' "http://127.0.0.1:9430/api/v1/stacks/preview?group=everything"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks/preview?group=everything", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"stack":{"id":"stack:example","layers":[]},"epoch":"a1b2c3d4e5f60718","seq":1}),
});
const data = await response.json();
```

**DELETE /api/v1/stacks/preview**

Stops the stack preview.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/stacks/preview?epoch=a1b2c3d4e5f60718"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks/preview?epoch=a1b2c3d4e5f60718", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `StackPreviewRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `stack` | `Stack` | The stack to preview. |
| `epoch` | `string` | The page's epoch: the editor page's session counter. |
| `seq` | `number` | The page's sequence number for this preview: a lower one is dropped. |

**Answer body**: `StackPreviewResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `applied` | `boolean` | False when the PUT was superseded by a newer one, or a newer epoch owns the preview; the answer is a 200 and the page stops resending that change. |
| `epoch` | `string` | The epoch that owns the preview after this PUT: the editor page's session counter. |
| `seq` | `number` | The highest sequence accepted for that epoch. |
| `layers` | `array of StackPreviewLayer` | What each layer of the previewed stack is doing. |
| `running` | `boolean` | True while the stack preview is running. |

| Query | Type | What it holds |
| --- | --- | --- |
| `group` | `string` | The id of a group in the active scene. The default scene's only group has the id everything. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `JSON object`.

The fields the daemon sends: whether a preview was dropped, and whether the page that sent the epoch owned it.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the request was handled. |
| `running` | `boolean` | True when a preview was dropped by this call. |
| `applied` | `boolean` | True when the page that sent the epoch owned the preview. |

| Query | Type | What it holds |
| --- | --- | --- |
| `epoch` | `string` | The page's epoch: the editor page's session counter. A preview stops only for the page whose epoch matches. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/patterns` | none | PatternsResponse |
| GET `/api/v1/patterns/{id}` | none | Pattern |
| PUT `/api/v1/patterns/{id}` | Pattern | Pattern |
| DELETE `/api/v1/patterns/{id}` | none | DeletePatternResponse |
| GET `/api/v1/stacks` | none | StacksResponse |
| GET `/api/v1/stacks/{id}` | none | StackDetail |
| PUT `/api/v1/stacks/{id}` | Stack | Stack |
| DELETE `/api/v1/stacks/{id}` | none | DeleteStackResponse |
| PUT `/api/v1/stacks/preview` | StackPreviewRequest | StackPreviewResponse |
| DELETE `/api/v1/stacks/preview` | none | JSON object |

## Example

**PUT**

Starts or updates the stack used by the preview engine, for a group id the active scene has.

```json
{
  "stack": {
    "id": "stack:example",
    "layers": []
  },
  "epoch": "a1b2c3d4e5f60718",
  "seq": 1
}
```

**DELETE**

Stops the stack preview.

This route takes no request body.
