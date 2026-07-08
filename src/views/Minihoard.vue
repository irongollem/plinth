<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { commands, events } from "../bindings";
import { useMinihoard } from "../composables/useMinihoard";
import { useToastStore } from "../stores/toastStore";

/* The minihoard console. minihoard speaks human text (no JSON mode), so
   this view is deliberately a terminal, not a data UI: preset buttons and
   a fetch box launch whitelisted subcommands; stdout/stderr stream in
   verbatim. Anything interactive (login, configure) belongs in a real
   terminal and is not offered here. */

const { info } = useMinihoard();
const toastStore = useToastStore();

type ConsoleLine = { text: string; isErr: boolean };
const lines = ref<ConsoleLine[]>([]);
const activeJobId = ref<string | null>(null);
/* The process can print before runMinihoard's response delivers the job
   id, so events can't be matched by id alone. The backend allows exactly
   one run at a time — while this view is busy, every event is ours. */
const launching = ref(false);
const busy = computed(() => launching.value || !!activeJobId.value);
const fetchInput = ref("");
const consoleEl = ref<HTMLElement | null>(null);

const currentMonth = () => {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;
};

const presets = [
  { label: "What's new", args: ["list", "--undownloaded"] },
  { label: "This month", args: ["list", "--month", currentMonth()] },
  { label: "Account", args: ["whoami"] },
  { label: "Folders", args: ["config"] },
];

const appendLine = (line: ConsoleLine) => {
  lines.value.push(line);
  // keep the console from growing unbounded over a long session
  if (lines.value.length > 2000)
    lines.value.splice(0, lines.value.length - 2000);
  nextTick(() => {
    consoleEl.value?.scrollTo({ top: consoleEl.value.scrollHeight });
  });
};

const run = async (args: string[]) => {
  if (!info.value || busy.value) return;
  launching.value = true;
  lines.value = [];
  appendLine({ text: `$ minihoard ${args.join(" ")}`, isErr: false });
  const result = await commands.runMinihoard(info.value.path, args);
  if (result.status === "ok") {
    // a fast command can finish before this response lands — the listener
    // already cleared launching in that case, and the run is over
    if (launching.value) activeJobId.value = result.data;
  } else {
    launching.value = false;
    toastStore.reportError("Failed to launch minihoard", result.error);
  }
};

const fetchTargets = async () => {
  const targets = fetchInput.value.trim().split(/\s+/).filter(Boolean);
  if (!targets.length) return;
  await run(["download", ...targets]);
};

const cancel = async () => {
  if (!activeJobId.value) return;
  await commands.cancelMinihoard(activeJobId.value);
};

let unlisten: (() => void) | undefined;
onMounted(async () => {
  unlisten = await events.minihoardStatus.listen((event) => {
    if (!busy.value) return; // stragglers from a cancelled run
    const status = event.payload;
    if ("Line" in status) {
      appendLine({ text: status.Line.line, isErr: status.Line.is_err });
    } else if ("Finished" in status) {
      activeJobId.value = null;
      launching.value = false;
      if (status.Finished.error) {
        appendLine({ text: status.Finished.error, isErr: true });
      }
      appendLine({
        text: status.Finished.success ? "— done —" : "— stopped —",
        isErr: !status.Finished.success,
      });
    }
  });
});
onUnmounted(() => unlisten?.());
</script>

<template>
  <div class="flex flex-col h-full min-h-0 p-6 gap-4">
    <div class="flex items-baseline gap-3">
      <h1 class="font-display text-[17px] tracking-wider">MINIHOARD</h1>
      <span class="font-mono text-[11px] text-base-content/40"
        >v{{ info?.version }}</span
      >
      <span
        class="font-mono text-[10.5px] text-base-content/30 truncate"
        :title="info?.path"
        >{{ info?.path }}</span
      >
    </div>
    <p class="text-[12.5px] text-base-content/60 -mt-2">
      Your MyMiniFactory hoard, from inside Plinth. Downloads land in
      minihoard's own folders — add them as a catalog folder to scan them in.
    </p>

    <div class="flex flex-wrap items-center gap-2">
      <button
        v-for="preset in presets"
        :key="preset.label"
        type="button"
        class="btn btn-sm"
        :disabled="busy"
        @click="run(preset.args)"
      >
        {{ preset.label }}
      </button>

      <span class="flex-1"></span>

      <input
        v-model="fetchInput"
        type="text"
        placeholder="object ids or names, e.g. 806054 or “dragon”"
        class="input input-sm input-bordered w-72 font-mono text-[12px]"
        :disabled="busy"
        @keydown.enter="fetchTargets"
      />
      <button
        type="button"
        class="btn btn-sm btn-primary"
        :disabled="busy || !fetchInput.trim()"
        @click="fetchTargets"
      >
        Fetch
      </button>
      <button
        v-if="activeJobId"
        type="button"
        class="btn btn-sm btn-error"
        @click="cancel"
      >
        Stop
      </button>
    </div>

    <div
      ref="consoleEl"
      class="flex-1 min-h-0 overflow-y-auto bg-base-300 border border-base-content/10 rounded-box p-3 font-mono text-[11.5px] leading-[1.65] whitespace-pre-wrap"
    >
      <div v-if="!lines.length" class="text-base-content/35">
        Pick an action above — output streams here, exactly as the CLI prints
        it.
      </div>
      <div
        v-for="(line, index) in lines"
        :key="index"
        :class="line.isErr ? 'text-base-content/45' : ''"
      >
        {{ line.text }}
      </div>
      <div v-if="busy" class="text-primary animate-pulse">▍</div>
    </div>
  </div>
</template>
