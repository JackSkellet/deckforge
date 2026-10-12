import { createRoot } from "react-dom/client";
import App from "./App";
async function start() {
  if (
    import.meta.env.DEV &&
    new URLSearchParams(location.search).get("fixture") === "1"
  ) {
    const { installFixture } = await import("./fixture");
    installFixture();
  }
  createRoot(document.getElementById("root")!).render(<App />);
}
void start();
