import { render } from "preact";
import "@xterm/xterm/css/xterm.css";
import "../../public/css/style.css";
import { App } from "./app";

const root = document.getElementById("app");
if (root) {
  render(<App />, root);
}
