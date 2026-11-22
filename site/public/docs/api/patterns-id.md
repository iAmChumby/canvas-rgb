# Pattern

Method and path: GET / PUT / DELETE /api/v1/patterns/{id}.

Area: Stacks and patterns.

## Request

**GET /api/v1/patterns/{id}**

Returns one pattern's frames and transitions.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/patterns/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/patterns/<id>", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/patterns/{id}**

Creates or updates a pattern.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"id":"pattern:Example","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#d21e28","#0f9d58","#1a73e8","#fbbc04"],"hold":0}],"motion":{"type":"none"}}' "http://127.0.0.1:9430/api/v1/patterns/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/patterns/<id>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"id":"pattern:Example","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#d21e28","#0f9d58","#1a73e8","#fbbc04"],"hold":0}],"motion":{"type":"none"}}),
});
const data = await response.json();
```

**DELETE /api/v1/patterns/{id}**

Deletes a pattern; one a group plays needs ?force=true, and the answer names the scenes that played it.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/patterns/<id>?force=true"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/patterns/<id>?force=true", {
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

**Answer body**: `Pattern`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the pattern, made from `pattern:` and the name. A save never renames a pattern, and a body whose id does not match the path is refused. |
| `built_on` | `BuiltOn` | The device shape the pattern was built on. |
| `frames` | `array of Frame` | The frames of the pattern, in order. |
| `motion` | `Motion` | How the frames move: `none` holds the first frame, `spin` turns the pattern along its LEDs at `turns_per_s` laps a second. |
| `playback` | `Playback` | How the frames play. Leave it out to loop. Optional. |

**PUT**

**Request body**: `Pattern`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the pattern, made from `pattern:` and the name. A save never renames a pattern, and a body whose id does not match the path is refused. |
| `built_on` | `BuiltOn` | The device shape the pattern was built on. |
| `frames` | `array of Frame` | The frames of the pattern, in order. |
| `motion` | `Motion` | How the frames move: `none` holds the first frame, `spin` turns the pattern along its LEDs at `turns_per_s` laps a second. Optional. |
| `playback` | `Playback` | How the frames play. Leave it out to loop. Optional. |

**Answer body**: `Pattern`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the pattern, made from `pattern:` and the name. A save never renames a pattern, and a body whose id does not match the path is refused. |
| `built_on` | `BuiltOn` | The device shape the pattern was built on. |
| `frames` | `array of Frame` | The frames of the pattern, in order. |
| `motion` | `Motion` | How the frames move: `none` holds the first frame, `spin` turns the pattern along its LEDs at `turns_per_s` laps a second. |
| `playback` | `Playback` | How the frames play. Leave it out to loop. Optional. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `DeletePatternResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the pattern was deleted. |
| `id` | `string` | The id of the pattern that was deleted. |
| `favorite_removed` | `boolean` | True when the delete also removed the pattern from the favourites. |
| `note` | `string` | The sentence about the group that went dark, when a group played the pattern: `pattern:Example was deleted while a group played it, so that group is dark`. It is left out when no group played the pattern. Optional. |

| Query | Type | What it holds |
| --- | --- | --- |
| `force` | `boolean` | True deletes a pattern that a scene still plays. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the pattern the route names. |

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

Returns one pattern's frames and transitions.

This route takes no request body.

**PUT**

Creates or updates a pattern.

```json
{
  "id": "pattern:Example",
  "built_on": {
    "led_count": 4,
    "arrangement": "ring"
  },
  "frames": [
    {
      "colors": [
        "#d21e28",
        "#0f9d58",
        "#1a73e8",
        "#fbbc04"
      ],
      "hold": 0
    }
  ],
  "motion": {
    "type": "none"
  }
}
```

**DELETE**

Deletes a pattern; one a group plays needs ?force=true, and the answer names the scenes that played it.

This route takes no request body.
