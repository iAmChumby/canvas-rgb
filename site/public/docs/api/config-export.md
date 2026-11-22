# Config export

Method and path: GET /api/v1/config/export.

Area: Settings and update.

## Request

**GET /api/v1/config/export**

Exports the configuration as one JSON bundle.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/config/export"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/config/export", {
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

**Answer body**: `ExportBundle`.

| Field | Type | What it holds |
| --- | --- | --- |
| `schema_version` | `number` | The format version of the saved file. |
| `exported_at` | `string` | The time the bundle was exported. |
| `app_version` | `string` | The version of the build that wrote the bundle. |
| `settings` | `Settings` | The saved application settings. |
| `layouts` | `LayoutsFile` | The saved layouts in the bundle. |
| `zones` | `ZonesFile` | The saved LED counts in the bundle. |
| `effects` | `EffectsFile` | The saved effects in the bundle. |
| `fans` | `FansFile` | The saved fan settings in the bundle. |
| `displays` | `array of DisplayConfig` | The numeric displays' configs. They are absent in bundles from before displays, and import then replaces the saved display configs with these, like the other files. |
| `stacks` | `array of Stack` | The saved effect stacks. They are absent in bundles from before stacks, and import then replaces the saved stacks with these, like the other files. |
| `lights` | `LightsFile` | The saved lights. They are absent in older bundles, and import then migrates their layouts. Optional. |
| `scenes` | `ScenesFile` | The saved scenes. They are absent in older bundles, as the lights are. Optional. |
| `patterns` | `array of Pattern` | The saved patterns. They are absent in older bundles. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/config/export` | none | ExportBundle |
| POST `/api/v1/config/import` | ExportBundle | JSON object |
| POST `/api/v1/first-run/dismiss-zone-panel` | none | JSON object |
| POST `/api/v1/onboarding/complete` | none | JSON object |
| GET `/api/v1/settings` | none | Settings |
| PUT `/api/v1/settings` | Settings | Settings |
| POST `/api/v1/update/install` | InstallUpdateRequest | InstallUpdateResponse |

## Example

**GET**

Exports the configuration as one JSON bundle.

This route takes no request body.
