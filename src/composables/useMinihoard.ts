import { ref } from "vue";
import { type MinihoardInfo, commands } from "../bindings";

/* The minihoard easter egg: Plinth's sibling CLI for fetching a
   MyMiniFactory library. Detection is module-scoped and runs once — the
   sidebar needs it to decide whether the menu exists at all, and the
   Minihoard view needs the same binary path to launch runs. */
const info = ref<MinihoardInfo | null>(null);
let detectStarted = false;

const detect = async () => {
  if (detectStarted) return;
  detectStarted = true;
  info.value = await commands.detectMinihoard();
};

export function useMinihoard() {
  return { info, detect };
}
