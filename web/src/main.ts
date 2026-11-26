import "./style.css";
import { BRAND } from "./brand";
import { renderHome } from "./pages/home";
import { renderProve } from "./pages/prove";
import { renderVerify } from "./pages/verify";
import { renderAbout } from "./pages/about";
import { renderExamples } from "./pages/examples";

const routes: Record<string, (root: HTMLElement) => void> = {
  "/": renderHome,
  "/prove": renderProve,
  "/verify": renderVerify,
  "/about": renderAbout,
  "/examples": renderExamples,
};

function route() {
  const root = document.getElementById("app")!;
  const path = location.hash.replace(/^#/, "") || "/";
  root.replaceChildren();
  (routes[path] ?? renderHome)(root);
  document.querySelectorAll("nav a").forEach((a) => {
    a.classList.toggle("active", a.getAttribute("href") === `#${path}`);
  });
  window.scrollTo(0, 0);
}

document.title = BRAND.name;
document.getElementById("brand-name")!.textContent = BRAND.name;
window.addEventListener("hashchange", route);
route();
