import { createApp } from "vue";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.vue";
import { applyTheme } from "./theme";

void applyTheme();
createApp(App).mount("#app");
