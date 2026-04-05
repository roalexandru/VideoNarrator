import { initMockApp } from "./mock-app";
import { initDownloads } from "./download";

// Prevent browser scroll restoration — always start at top
if ("scrollRestoration" in history) {
  history.scrollRestoration = "manual";
}
window.scrollTo(0, 0);

// Scroll-triggered fade-in animations
const observer = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        entry.target.classList.add("visible");
        observer.unobserve(entry.target);
      }
    }
  },
  { threshold: 0.1 }
);

document.querySelectorAll(".fade-in").forEach((el) => observer.observe(el));

// Mobile menu toggle
const menuButton = document.getElementById("mobile-menu-button");
const mobileMenu = document.getElementById("mobile-menu");

menuButton?.addEventListener("click", () => {
  mobileMenu?.classList.toggle("hidden");
});

// Initialize mock app demo
initMockApp();

// Initialize smart download section
initDownloads();
