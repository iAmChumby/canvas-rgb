# Effect draft

Method and path: GET / PUT / DELETE /api/v1/effects/draft.

Area: Effects.

## Request

**GET /api/v1/effects/draft**

Reports the effect draft playing on the lights and its remaining lease.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/draft"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/draft", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/effects/draft**

Starts or renews an effect, stack or pattern draft on the lights.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","props":{}}' "http://127.0.0.1:9430/api/v1/effects/draft"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/draft", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"html":"<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>","props":{}}),
});
const data = await response.json();
```

**DELETE /api/v1/effects/draft**

Ends the current effect draft.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/draft?epoch=a1b2c3d4e5f60718"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/draft?epoch=a1b2c3d4e5f60718", {
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

**Answer body**: `DraftStatus`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `boolean` | True while a draft is on the lights. |
| `id` | `string` | The id of the draft effect, made from `draft:` and the source hash. Optional. |
| `source_hash` | `string` | The hash of the HTML the draft was built from. Optional. |
| `props` | `an object keyed by text, with PropValue values` | The property values the draft runs with. Empty when no draft runs. |
| `lease_remaining_ms` | `number` | Milliseconds left in the lease; 0 when no draft runs. A PUT renews it. |
| `heartbeat_ms` | `number` | How often a client PUTs to keep the draft alive. |
| `lease_ms` | `number` | How long the draft survives without a PUT. |
| `note` | `string` | A sentence about the ending, when it needs saying: the prior effect could not be restored, so the lights are black. Optional. |
| `applied` | `boolean` | Whether the PUT left a draft on the lights. A stack draft that a newer epoch or sequence superseded answers false, and a DELETE that carries an older epoch answers false and leaves the draft running. Optional. |
| `epoch` | `string` | The epoch that owns a stack draft, when one is running: the editor page's session counter. Optional. |
| `seq` | `number` | The highest sequence accepted for that epoch. Optional. |

**PUT**

**Request body**: `DraftRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `html` | `string` | The draft HTML, at most 1 MiB of bytes. A stack draft leaves it out. Optional. |
| `stack` | `Stack` | A whole stack as the draft. It plays on the lights, with its layer settings from this document and its property values under `props`. Optional. |
| `pattern` | `Pattern` | A pattern as the draft. Exactly one of `html`, `stack` and `pattern` is set. A pattern draft needs `epoch` and `seq`, as a stack draft does. Optional. |
| `epoch` | `string` | The page's epoch: the editor page's session counter. A stack draft needs it. Optional. |
| `seq` | `number` | The page's sequence number in that epoch; a dropped PUT answers 200 with applied false. Optional. |
| `props` | `an object keyed by text, with PropValue values` | The property values the draft runs with. |

**Answer body**: `DraftStatus`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `boolean` | True while a draft is on the lights. |
| `id` | `string` | The id of the draft effect, made from `draft:` and the source hash. Optional. |
| `source_hash` | `string` | The hash of the HTML the draft was built from. Optional. |
| `props` | `an object keyed by text, with PropValue values` | The property values the draft runs with. Empty when no draft runs. |
| `lease_remaining_ms` | `number` | Milliseconds left in the lease; 0 when no draft runs. A PUT renews it. |
| `heartbeat_ms` | `number` | How often a client PUTs to keep the draft alive. |
| `lease_ms` | `number` | How long the draft survives without a PUT. |
| `note` | `string` | A sentence about the ending, when it needs saying: the prior effect could not be restored, so the lights are black. Optional. |
| `applied` | `boolean` | Whether the PUT left a draft on the lights. A stack draft that a newer epoch or sequence superseded answers false, and a DELETE that carries an older epoch answers false and leaves the draft running. Optional. |
| `epoch` | `string` | The epoch that owns a stack draft, when one is running: the editor page's session counter. Optional. |
| `seq` | `number` | The highest sequence accepted for that epoch. Optional. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `DraftStatus`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `boolean` | True while a draft is on the lights. |
| `id` | `string` | The id of the draft effect, made from `draft:` and the source hash. Optional. |
| `source_hash` | `string` | The hash of the HTML the draft was built from. Optional. |
| `props` | `an object keyed by text, with PropValue values` | The property values the draft runs with. Empty when no draft runs. |
| `lease_remaining_ms` | `number` | Milliseconds left in the lease; 0 when no draft runs. A PUT renews it. |
| `heartbeat_ms` | `number` | How often a client PUTs to keep the draft alive. |
| `lease_ms` | `number` | How long the draft survives without a PUT. |
| `note` | `string` | A sentence about the ending, when it needs saying: the prior effect could not be restored, so the lights are black. Optional. |
| `applied` | `boolean` | Whether the PUT left a draft on the lights. A stack draft that a newer epoch or sequence superseded answers false, and a DELETE that carries an older epoch answers false and leaves the draft running. Optional. |
| `epoch` | `string` | The epoch that owns a stack draft, when one is running: the editor page's session counter. Optional. |
| `seq` | `number` | The highest sequence accepted for that epoch. Optional. |

| Query | Type | What it holds |
| --- | --- | --- |
| `epoch` | `string` | The page's epoch: the editor page's session counter. A delete from a page that does not own the draft answers applied false and leaves the draft running. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/effects` | none | EffectsResponse |
| GET `/api/v1/effects/{id}` | none | EffectDetail |
| PUT `/api/v1/effects/{id}` | EffectDetail | JSON object |
| DELETE `/api/v1/effects/{id}` | none | DeleteEffectResponse |
| POST `/api/v1/effects/{id}/activate` | ActivateEffect | JSON object |
| PATCH `/api/v1/effects/{id}/defaults` | Props | JSON object |
| PUT `/api/v1/effects/{id}/favorite` | SetFavorite | JSON object |
| POST `/api/v1/effects/{id}/presets` | PresetCreate | PresetsResponse |
| PUT `/api/v1/effects/{id}/presets/{name}` | PresetReplace | PresetsResponse |
| DELETE `/api/v1/effects/{id}/presets/{name}` | none | PresetsResponse |
| GET `/api/v1/effects/{id}/source` | none | EffectSource |
| GET `/api/v1/effects/{id}/thumbnail` | none | Binary thumbnail |
| GET `/api/v1/effects/draft` | none | DraftStatus |
| PUT `/api/v1/effects/draft` | DraftRequest | DraftStatus |
| DELETE `/api/v1/effects/draft` | none | DraftStatus |
| POST `/api/v1/effects/import` | ImportEffect | ImportEffectResponse |
| POST `/api/v1/effects/preview` | EffectPreviewRequest | EffectPreviewResponse |

## Example

**GET**

Reports the effect draft playing on the lights and its remaining lease.

This route takes no request body.

**PUT**

Starts or renews an effect, stack or pattern draft on the lights.

```json
{
  "html": "<head><title>Demo</title><meta description=\"A short demo.\" /></head><body><canvas id=\"stage\" width=\"320\" height=\"200\"></canvas></body><script>function draw(){ window.requestAnimationFrame(draw); } window.requestAnimationFrame(draw);</script>",
  "props": {}
}
```

**DELETE**

Ends the current effect draft.

This route takes no request body.
