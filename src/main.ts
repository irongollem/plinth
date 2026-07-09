// latin-* subsets only — the UI is English-only, and the general (no
// subset) imports pull in cyrillic/vietnamese/etc. glyphs nothing uses.
import "@fontsource/archivo/latin-400.css";
import "@fontsource/archivo/latin-500.css";
import "@fontsource/archivo/latin-600.css";
import "@fontsource/archivo/latin-700.css";
import "@fontsource/archivo/latin-800.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "@fontsource/ibm-plex-mono/latin-600.css";
import "@fontsource/bebas-neue/latin-400.css";
import "@fontsource/cormorant-garamond/latin-600.css";
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
