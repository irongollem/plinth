import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import "./assets/styles.css";
import { useThemeStore } from "./stores/themeStore";

const app = createApp(App);
app.use(createPinia());
// Applied before mount so the correct theme is set on first paint, not
// flashed from the daisyUI default after Vue takes over.
useThemeStore();
app.mount("#app");
