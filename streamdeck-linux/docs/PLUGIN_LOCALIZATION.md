# Plugin language

Choose **Preferences → Startup & about → Plugin language** and save the language
you want. A full Deckforge restart applies it. Saving the preference does not
interrupt running plugins, change their language mid-session or reopen inspectors.
Deckforge's own menus currently remain English.

The supported plugin languages are German, English, Spanish, French, Japanese,
Korean, Simplified Chinese and Traditional Chinese. **System language** reads the
first nonempty `LC_ALL`, `LC_MESSAGES`, then `LANG` value. Locale suffixes such as
`.UTF-8` are normalized; unsupported locales use English. Chinese script aliases
take precedence over region, with Taiwan, Hong Kong and Macao selecting Traditional
Chinese and other Chinese locales selecting Simplified Chinese. This normalization
is Deckforge's Linux policy.

Both Node/native plugin launch information and property-inspector registration
receive the same active `application.language`, even if a plugin has no matching
translation file. Installing or restarting one plugin uses the current active
language; it does not apply a pending application preference.

## Manifest labels

Deckforge reads `<language>.json` from the plugin root, or `locales/` when the root
file is absent. The display copy overrides only the documented text fields:

- Plugin name and description.
- Action names and tooltips, indexed by the original action UUID.
- State names, within the existing state count.
- Encoder `TriggerDescription` text for rotate, push, touch and long touch.

Missing or empty translations retain the manifest text. The host does not fall
back to `en.json` when another language file is missing. Category, action IDs,
paths, settings, titles and live plugin output are not translated. Default slot
labels equal to the original action name receive the translated display name;
other saved labels remain literal. Source files and exported profile data remain
unchanged.

The catalogue keeps original declaration defaults separately from display labels.
New Multi Action, Dial Stack and Action Trigger children use those originals when
saved, so changing language cannot turn a translated default into a permanent
custom label. Their editor displays the localized name while retaining explicit
custom labels and raw state metadata.

Plugin-owned `Localization` dictionaries and inspector HTML are handled by the
plugin itself. For example, the unchanged Elgato SDK has its own custom-string
fallback through English. Deckforge supplies the language and preserves the plugin
working directory; it does not rewrite vendor JavaScript or DOM text.

## Invalid resources and validation

Language files must resolve inside the plugin directory, be regular files and
fit within 1 MiB. Missing files need no warning. Invalid files retain the original
manifest; invalid known text fields are ignored while valid fields remain usable.
A warning appears in the plugin's runtime details. This per-field error policy is
an independent choice because the original app's exact malformed-field handling
has not been established.

The field contract follows Elgato's
[localization guide](https://docs.elgato.com/streamdeck/sdk/guides/i18n/) and
[registration reference](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/).
Bounded static inspection of the pinned Windows 7.6 executable corroborates
nonempty-string overrides, root/locales lookup and manifest fallback.

`tests/plugin_localization_interop.py` uses unchanged official Counter resources
and the unchanged SDK 2.0.4, a real native property inspector, and temporary
profiles. It checks labels, custom-string fallback, language consistency, dynamic
text preservation and identical export contents across a language change.
`frontend/tests/native_plugin_language_interop.py` runs two isolated native app
lifetimes to check retained drafts, stale-save protection, pending language and
application after restart. Rust tests cover resource limits and fallback rules.
