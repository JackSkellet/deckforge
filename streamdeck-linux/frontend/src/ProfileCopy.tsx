import { Field } from "./Fields";
import { Modal } from "./Panels";
import type { DeviceRecord } from "./model";
import { deviceOptionLabel } from "./device";
export interface ProfileCopyDraft {
  operation_id: string;
  device: string;
  profile: string;
  source_name: string;
  devices: DeviceRecord[];
  target_device: string;
  name: string;
  simulate: boolean;
}
export function eligibleProfileTargets(
  devices: DeviceRecord[],
  source: string,
) {
  return devices.filter(
    (device) => device.id !== source && device.profile_compatible === true,
  );
}
export function ProfileCopyDialog({
  draft,
  busy,
  error,
  onChange,
  onSubmit,
  onClose,
}: {
  draft: ProfileCopyDraft;
  busy: boolean;
  error: string;
  onChange: (draft: ProfileCopyDraft) => void;
  onSubmit: () => void;
  onClose: () => void;
}) {
  const targets = eligibleProfileTargets(draft.devices, draft.device);
  return (
    <Modal title="Copy profile to device" onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <p>
          Copy “{draft.source_name}” to another device. The copy activates
          there; this editor stays on its current device.
        </p>
        <p className="hint">The original profile stays unchanged.</p>
        {targets.length === 0 ? (
          <p className="warning" role="status">
            No other compatible devices are associated with this profile.
          </p>
        ) : null}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <fieldset
          className="profile-copy-fields"
          disabled={busy || targets.length === 0}
        >
          <Field label="Target device">
            <select
              autoFocus
              value={draft.target_device}
              onChange={(event) =>
                onChange({ ...draft, target_device: event.target.value })
              }
              required
            >
              {targets.length === 0 && (
                <option value="">No compatible device</option>
              )}
              {targets.map((device) => (
                <option
                  key={device.id}
                  value={device.id}
                  title={`${device.name} · ${device.id}`}
                >
                  {deviceOptionLabel(device, draft.devices, draft.simulate)}
                </option>
              ))}
            </select>
          </Field>
          <Field
            label="Copy name"
            hint="Leave empty to use the default copy name."
          >
            <input
              value={draft.name}
              onChange={(event) =>
                onChange({ ...draft, name: event.target.value })
              }
            />
          </Field>
        </fieldset>
        <div className="row">
          <button
            className="primary"
            type="submit"
            disabled={
              busy ||
              !targets.some((device) => device.id === draft.target_device)
            }
          >
            {busy ? "Copying…" : "Copy profile"}
          </button>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
        </div>
      </form>
    </Modal>
  );
}
