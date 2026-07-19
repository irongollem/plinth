<script setup lang="ts">
import { nextTick, ref, useTemplateRef, watch } from "vue";

const props = defineProps<{
  label: string;
  kind: "designer" | "release";
  count?: number;
  date?: string | null;
  editable?: boolean;
  rename?: (newName: string) => Promise<boolean>;
}>();

const editing = ref(false);
const busy = ref(false);
const draft = ref("");
const input = useTemplateRef<HTMLInputElement>("input");

watch(
  () => props.label,
  (label) => {
    draft.value = label;
    editing.value = false;
  },
);

const startEditing = async () => {
  if (!props.editable || busy.value) return;
  draft.value = props.label;
  editing.value = true;
  await nextTick();
  input.value?.select();
};

const cancel = () => {
  if (busy.value) return;
  editing.value = false;
  draft.value = props.label;
};

const submit = async () => {
  const next = draft.value.trim();
  if (!next || busy.value) return;
  if (next === props.label) {
    cancel();
    return;
  }
  busy.value = true;
  try {
    if (await props.rename?.(next)) editing.value = false;
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div class="group flex min-w-0 items-center gap-2">
    <form
      v-if="editing"
      class="flex min-w-0 items-center gap-1"
      @submit.prevent="submit"
    >
      <input
        ref="input"
        v-model="draft"
        class="input input-xs h-7 min-w-36 max-w-72 text-xs"
        :class="kind === 'release' ? 'font-mono uppercase' : 'font-semibold'"
        :aria-label="`New ${kind} name`"
        :disabled="busy"
        @keydown.escape.prevent="cancel"
      />
      <button
        type="submit"
        class="btn btn-ghost btn-xs h-7 min-h-0 w-7 p-0"
        :disabled="busy || !draft.trim()"
        :aria-label="`Save ${kind} name`"
        title="Save"
      >
        <span v-if="busy" class="loading loading-spinner loading-xs"></span>
        <span v-else aria-hidden="true">✓</span>
      </button>
      <button
        type="button"
        class="btn btn-ghost btn-xs h-7 min-h-0 w-7 p-0"
        :disabled="busy"
        :aria-label="`Cancel ${kind} rename`"
        title="Cancel"
        @click="cancel"
      >
        <span aria-hidden="true">×</span>
      </button>
    </form>

    <template v-else>
      <span
        class="truncate"
        :class="
          kind === 'designer'
            ? 'font-bold text-[13px]'
            : 'font-mono font-semibold text-[10px] tracking-widest uppercase text-base-content/50'
        "
        >{{ label }}</span
      >
      <button
        v-if="editable"
        type="button"
        class="btn btn-ghost btn-xs h-6 min-h-0 w-6 shrink-0 p-0 text-base-content/45 opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100"
        :aria-label="`Rename ${kind} ${label}`"
        :title="`Rename ${kind} for every assigned model`"
        @click="startEditing"
      >
        <span aria-hidden="true">✎</span>
      </button>
      <span
        v-if="kind === 'designer' && count !== undefined"
        class="font-mono text-[10px] text-base-content/40"
      >
        {{ count }} model{{ count === 1 ? "" : "s" }}
      </span>
      <span
        v-if="kind === 'release' && date"
        class="font-mono text-[9.5px] text-base-content/35"
        >{{ date }}</span
      >
    </template>
  </div>
</template>
