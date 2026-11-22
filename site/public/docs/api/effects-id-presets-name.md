# Effect preset

Method and path: PUT / DELETE /api/v1/effects/{id}/presets/{name}.

Area: Effects.

## Request

**PUT /api/v1/effects/{id}/presets/{name}**

Replaces or renames one saved preset.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"rename_to":null,"props":{"speed":{"type":"number","value":40}},"expected":{"speed":{"type":"number","value":30}}}' "http://127.0.0.1:9430/api/v1/effects/<id>/presets/<name>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/presets/<name>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"rename_to":null,"props":{"speed":{"type":"number","value":40}},"expected":{"speed":{"type":"number","value":30}}}),
});
const data = await response.json();
```

**DELETE /api/v1/effects/{id}/presets/{name}**

Deletes one saved preset.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/<id>/presets/<name>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/presets/<name>", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `PresetReplace`.

| Field | Type | What it holds |
| --- | --- | --- |
| `rename_to` | `string or null` | The new name for the preset; null keeps the name it has. Optional. |
| `props` | `an object keyed by text, with PropValue values` | The property values the preset saves, by property key. |
| `expected` | `an object keyed by text, with PropValue values` | The property values the client last saw for this preset. |

**Answer body**: `PresetsResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `user_presets` | `array of Preset` | The user's saved presets. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `PresetsResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `user_presets` | `array of Preset` | The user's saved presets. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect the preset belongs to. |
| `name` | `string` | The name of the saved preset to change. |

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

**PUT**

Replaces or renames one saved preset.

```json
{
  "rename_to": null,
  "props": {
    "speed": {
      "type": "number",
      "value": 40
    }
  },
  "expected": {
    "speed": {
      "type": "number",
      "value": 30
    }
  }
}
```

**DELETE**

Deletes one saved preset.

This route takes no request body.
