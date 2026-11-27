# Stack

Method and path: GET / PUT / DELETE /api/v1/stacks/{id}.

Area: Stacks and patterns.

## Request

**GET /api/v1/stacks/{id}**

Returns one stack and the state of each of its layers.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/stacks/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks/<id>", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/stacks/{id}**

Creates or replaces a stack.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"id":"stack:example","layers":[]}' "http://127.0.0.1:9430/api/v1/stacks/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks/<id>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"id":"stack:example","layers":[]}),
});
const data = await response.json();
```

**DELETE /api/v1/stacks/{id}**

Deletes a stack; one a group plays needs ?force=true.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/stacks/<id>?force=true"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks/<id>?force=true", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `StackDetail`.

| Field | Type | What it holds |
| --- | --- | --- |
| `stack` | `Stack` | The stack itself. |
| `layers` | `array of StackLayerStatus` | The state of each layer of the stack. |

**PUT**

**Request body**: `Stack`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the stack, made from `stack:` and the name. The name is the id without the prefix, so there is one string. |
| `layers` | `array of StackLayer` | The layers of the stack, bottom first: the first layer is painted first. Optional. |

**Answer body**: `Stack`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the stack, made from `stack:` and the name. The name is the id without the prefix, so there is one string. |
| `layers` | `array of StackLayer` | The layers of the stack, bottom first: the first layer is painted first. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `DeleteStackResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the stack was deleted. |
| `id` | `string` | The id of the stack that was deleted. |
| `presets_removed` | `number` | How many presets of the stack went with it. |
| `favorite_removed` | `boolean` | True when the stack was one of the person's favourites. |
| `note` | `string` | Set when the deleted stack was on the lights and they went black. The text says the stack was deleted while a group played it, so that group is dark. Optional. |

| Query | Type | What it holds |
| --- | --- | --- |
| `force` | `boolean` | True deletes a stack that a scene still plays. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the stack the route names. |

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

**GET**

Returns one stack and the state of each of its layers.

This route takes no request body.

**PUT**

Creates or replaces a stack.

```json
{
  "id": "stack:example",
  "layers": []
}
```

**DELETE**

Deletes a stack; one a group plays needs ?force=true.

This route takes no request body.
