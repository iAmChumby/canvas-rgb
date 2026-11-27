# Stacks

Method and path: GET /api/v1/stacks.

Area: Stacks and patterns.

## Request

**GET /api/v1/stacks**

Lists the saved stacks.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/stacks"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/stacks", {
  method: "GET",
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

**Answer body**: `StacksResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `stacks` | `array of StackSummary` | The saved stacks. |

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

Lists the saved stacks.

This route takes no request body.
