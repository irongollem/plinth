import { onUnmounted, ref } from "vue";

/**
 * 6-DOF input from a 3D mouse (3Dconnexion SpaceMouse / SpaceNavigator).
 *
 * The Tauri webview is not Chromium everywhere — WebHID (the richest way to
 * talk to a SpaceMouse) is absent on the macOS/Linux WebKit backends. The
 * Gamepad API, by contrast, is supported across all of them, and the
 * 3Dconnexion driver publishes the puck as a standard game controller whose
 * axes are the six degrees of freedom. So we poll gamepads rather than open
 * an HID channel: no permission prompt, no device picker, works everywhere.
 *
 * Axis layout reported by the 3Dconnexion driver (values in [-1, 1]):
 *   axes[0]  translate X   (slide right +)
 *   axes[1]  translate Y   (push forward +)
 *   axes[2]  translate Z   (lift up +)
 *   axes[3]  rotate  X     (pitch / tilt)
 *   axes[4]  rotate  Y     (roll)
 *   axes[5]  rotate  Z     (yaw / spin)
 * Consumers decide what each maps to; this composable only cleans the signal
 * (deadzone + sensitivity) and delivers it once per animation frame.
 */

export interface SpaceMouseMotion {
  /** Translation, each axis in roughly [-1, 1] after deadzone/sensitivity. */
  tx: number;
  ty: number;
  tz: number;
  /** Rotation, each axis in roughly [-1, 1] after deadzone/sensitivity. */
  rx: number;
  ry: number;
  rz: number;
}

/** True when the gamepad id looks like a 3Dconnexion device. */
export const isSpaceMouse = (pad: Pick<Gamepad, "id">): boolean => {
  const id = pad.id.toLowerCase();
  return (
    id.includes("3dconnexion") ||
    id.includes("spacemouse") ||
    id.includes("space mouse") ||
    id.includes("spacenavigator") ||
    id.includes("space navigator") ||
    id.includes("spacepilot") ||
    id.includes("space pilot") ||
    // 3Dconnexion USB vendor ids, as they appear in the gamepad id string
    id.includes("vendor: 256f") ||
    id.includes("vendor: 046d")
  );
};

// Below this the axis is treated as centered — a real puck rests with a
// little noise on every axis, and without a floor the model drifts forever.
const DEADZONE = 0.06;

export const cleanAxis = (value: number): number => {
  if (Math.abs(value) < DEADZONE) return 0;
  // Rescale so motion ramps from 0 at the deadzone edge, not from a jump
  const scaled = (Math.abs(value) - DEADZONE) / (1 - DEADZONE);
  return Math.sign(value) * scaled;
};

export function useSpaceMouse(onMotion: (motion: SpaceMouseMotion) => void) {
  const connected = ref(false);
  /** Multiplies every axis before it reaches the consumer. */
  const sensitivity = ref(1);

  let padIndex: number | null = null;
  let frame = 0;

  const findPad = (): Gamepad | null => {
    if (!navigator.getGamepads) return null;
    for (const pad of navigator.getGamepads()) {
      if (pad && isSpaceMouse(pad)) return pad;
    }
    return null;
  };

  const poll = () => {
    frame = requestAnimationFrame(poll);
    const pads = navigator.getGamepads?.();
    const pad = padIndex !== null && pads ? pads[padIndex] : null;
    if (!pad) {
      // Device was reindexed or unplugged mid-stream — try to re-find it
      const found = findPad();
      padIndex = found ? found.index : null;
      connected.value = padIndex !== null;
      return;
    }

    const s = sensitivity.value;
    const a = pad.axes;
    const motion: SpaceMouseMotion = {
      tx: cleanAxis(a[0] ?? 0) * s,
      ty: cleanAxis(a[1] ?? 0) * s,
      tz: cleanAxis(a[2] ?? 0) * s,
      rx: cleanAxis(a[3] ?? 0) * s,
      ry: cleanAxis(a[4] ?? 0) * s,
      rz: cleanAxis(a[5] ?? 0) * s,
    };
    // Skip the callback (and the render it triggers) on a resting puck
    if (
      motion.tx ||
      motion.ty ||
      motion.tz ||
      motion.rx ||
      motion.ry ||
      motion.rz
    ) {
      onMotion(motion);
    }
  };

  const start = () => {
    if (frame) return;
    const found = findPad();
    padIndex = found ? found.index : null;
    connected.value = padIndex !== null;
    frame = requestAnimationFrame(poll);
  };

  const stop = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    padIndex = null;
    connected.value = false;
  };

  const onConnect = (e: GamepadEvent) => {
    if (isSpaceMouse(e.gamepad)) {
      padIndex = e.gamepad.index;
      connected.value = true;
    }
  };
  const onDisconnect = (e: GamepadEvent) => {
    if (e.gamepad.index === padIndex) {
      padIndex = null;
      connected.value = false;
    }
  };

  window.addEventListener("gamepadconnected", onConnect);
  window.addEventListener("gamepaddisconnected", onDisconnect);
  start();

  onUnmounted(() => {
    stop();
    window.removeEventListener("gamepadconnected", onConnect);
    window.removeEventListener("gamepaddisconnected", onDisconnect);
  });

  return { connected, sensitivity, start, stop };
}
