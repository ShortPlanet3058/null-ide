// Null's site: the hero's scenes, words lit as you scroll, the story played by scrolling,
// things arriving as they come into view, the platform you're on, the film over the page.
(() => {
  "use strict";
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => [...root.querySelectorAll(s)];
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));

  // ---------- the nav: solid once the page moves ----------
  const nav = $("#nav");
  const solid = () => nav.classList.toggle("solid", scrollY > 40);
  addEventListener("scroll", solid, { passive: true });
  solid();

  // ---------- the hero: one scene after another ----------
  const slides = $$(".slide"), copies = $$(".copy"), tabs = $("#hero-tabs");
  const SCENE = 7000;
  let current = 0, started = performance.now(), paused = false;
  slides.forEach((slide, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "tab");
    b.innerHTML = `${slide.dataset.label}<span class="bar"><i></i></span>`;
    b.addEventListener("click", () => go(i));
    tabs.appendChild(b);
  });
  const bars = $$(".bar i", tabs), buttons = $$("button", tabs);
  function go(i) {
    current = (i + slides.length) % slides.length;
    started = performance.now();
    slides.forEach((s, k) => {
      const on = k === current, video = $("video", s);
      s.classList.toggle("on", on);
      if (on && !still) {
        video.preload = "auto";
        video.currentTime = 0;
        video.play().catch(() => {});
      } else if (!on) {
        setTimeout(() => { if (!s.classList.contains("on")) video.pause(); }, 1200);
      }
      // The next one, ready before it's needed.
      if (k === (current + 1) % slides.length) video.preload = "auto";
    });
    copies.forEach((c, k) => c.classList.toggle("on", k === current));
    buttons.forEach((b, k) => b.setAttribute("aria-selected", k === current));
    bars.forEach((b, k) => { b.style.width = k < current ? "100%" : "0"; });
  }
  function tick(now) {
    if (!paused && !still) {
      const p = clamp((now - started) / SCENE);
      bars[current].style.width = `${p * 100}%`;
      if (p >= 1) go(current + 1);
    }
    requestAnimationFrame(tick);
  }
  // Only while the hero is seen, and the page is shown.
  new IntersectionObserver(([e]) => { paused = !e.isIntersecting; if (!paused) started = performance.now() - parseFloat(bars[current].style.width || 0) / 100 * SCENE; }, { threshold: 0.2 }).observe($("#hero"));
  document.addEventListener("visibilitychange", () => { paused = document.hidden; });
  go(0);
  requestAnimationFrame(tick);

  // A soft amber light that follows the pointer over the hero.
  const glow = $("#glow"), hero = $("#hero");
  hero.addEventListener("pointermove", e => {
    const r = hero.getBoundingClientRect();
    glow.style.left = `${e.clientX - r.left}px`;
    glow.style.top = `${e.clientY - r.top}px`;
  });

  // ---------- the platform you're on ----------
  const ua = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent;
  const os = /mac/i.test(ua) && !/iphone|ipad/i.test(navigator.userAgent) ? "mac" : /win/i.test(ua) ? "windows" : /linux/i.test(ua) && !/android/i.test(navigator.userAgent) ? "linux" : null;
  if (os) {
    const mine = $(`.platform[data-os="${os}"]`);
    mine.classList.add("mine");
    const name = { mac: "macOS", windows: "Windows", linux: "Linux" }[os];
    $("#hero-download").textContent = `Download for ${name}`;
    $("#hero-download").href = mine.href;
  }
  // The second download in a card (a link inside a link isn't allowed: a span that goes).
  $$(".platform .alt").forEach(alt => alt.addEventListener("click", e => {
    e.preventDefault();
    location.href = alt.dataset.href;
  }));
  // Keys as this computer names them.
  if (os && os !== "mac") {
    $$(".lead, .steps span, .three p, .three .k").forEach(el => {
      el.innerHTML = el.innerHTML.replace(/⌘/g, "Ctrl+");
    });
  }

  // ---------- words lit as you scroll ----------
  const scrub = $("#scrub");
  const KEY = new Set(["nothing,", "screen", "back", "code."]);
  scrub.innerHTML = scrub.textContent.split(/(\s+)/).map(w => (/\s/.test(w) ? w : `<span${KEY.has(w) ? ' class="key"' : ""}>${w}</span>`)).join("");
  const words = $$("span", scrub);
  function lightWords() {
    const r = scrub.getBoundingClientRect();
    const p = clamp((innerHeight * 0.85 - r.top) / (r.height + innerHeight * 0.35));
    const lit = Math.round(p * words.length);
    words.forEach((w, i) => w.classList.toggle("lit", i < lit));
  }

  // ---------- the story, played by scrolling ----------
  const story = $("#story"), frame = $("#film"), box = $(".story-frame"), storyBar = $("#story-bar");
  const steps = $$("#steps li");
  const B = k => 33.66 + 3.2 * k;            // the film's bars (scripts/promo/promo.html)
  const FROM = B(0), TO = B(15) - 0.3;
  let film = null, wanted = FROM, drawn = -1;
  // Loaded when the story comes near: it's the film's own engine, drawing each moment.
  new IntersectionObserver(([e], obs) => {
    if (!e.isIntersecting) return;
    frame.src = frame.dataset.src;
    obs.disconnect();
  }, { rootMargin: "100% 0px" }).observe(story);
  addEventListener("message", e => {
    if (e.data === "null-film-ready" && e.source === frame.contentWindow) {
      film = frame.contentWindow;
      box.classList.add("ready");
      drawn = -1;
    }
  });
  const wide = () => innerWidth > 960;
  function storyTime() {
    if (wide()) {
      const r = story.getBoundingClientRect();
      return FROM + clamp(-r.top / (r.height - innerHeight)) * (TO - FROM);
    }
    // On a phone the story plays by itself while it's on screen.
    return FROM + ((performance.now() / 1000) % (TO - FROM));
  }
  steps.forEach(li => li.addEventListener("click", () => {
    const at = B(+li.dataset.at) + 0.05;
    if (!wide()) { phoneStart = performance.now() / 1000 - (at - FROM); return; }
    const top = story.offsetTop + (at - FROM) / (TO - FROM) * (story.offsetHeight - innerHeight);
    scrollTo({ top, behavior: still ? "auto" : "smooth" });
  }));
  let phoneStart = 0;
  function drawStory() {
    let t = storyTime();
    if (!wide()) t = FROM + ((performance.now() / 1000 - phoneStart) % (TO - FROM));
    if (film && Math.abs(t - drawn) > 0.004) {
      try { film.render(t); drawn = t; } catch (_) { /* not ready yet */ }
    }
    storyBar.style.width = `${(t - FROM) / (TO - FROM) * 100}%`;
    let on = 0;
    steps.forEach((li, i) => { if (t >= B(+li.dataset.at)) on = i; });
    steps.forEach((li, i) => { li.classList.toggle("on", i === on); li.classList.toggle("done", i < on); });
  }

  // ---------- things arriving as they come into view ----------
  const seen = new IntersectionObserver(entries => {
    entries.forEach(e => {
      if (!e.isIntersecting) return;
      e.target.classList.add("in");
      seen.unobserve(e.target);
    });
  }, { threshold: 0.12, rootMargin: "0px 0px -6% 0px" });
  $$(".reveal").forEach((el, i) => {
    // Neighbours in a row arrive one after the other.
    el.style.transitionDelay = `${(i % 3) * 80}ms`;
    seen.observe(el);
  });

  // Numbers counting up, once.
  const counted = new IntersectionObserver(([e], obs) => {
    if (!e.isIntersecting) return;
    obs.disconnect();
    $$("[data-count]").forEach(el => {
      const end = +el.dataset.count, t0 = performance.now();
      const step = now => {
        const p = still ? 1 : clamp((now - t0) / 1600);
        const v = Math.round(end * (1 - Math.pow(1 - p, 3)));
        el.textContent = v.toLocaleString("en-US");
        if (p < 1) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
  }, { threshold: 0.4 });
  counted.observe($("#numbers"));

  // ---------- one loop for what follows the scroll ----------
  function frameLoop() {
    lightWords();
    const r = story.getBoundingClientRect();
    if (r.bottom > -200 && r.top < innerHeight + 200) drawStory();
    requestAnimationFrame(frameLoop);
  }
  requestAnimationFrame(frameLoop);

  // ---------- the film, over the page ----------
  const modal = $("#modal"), modalVideo = $("#modal-video");
  function openFilm() {
    modal.hidden = false;
    requestAnimationFrame(() => modal.classList.add("open"));
    modalVideo.play().catch(() => {});
    $(".modal-close").focus();
  }
  function closeFilm() {
    modal.classList.remove("open");
    modalVideo.pause();
    setTimeout(() => { modal.hidden = true; }, 400);
  }
  $$("[data-film]").forEach(b => b.addEventListener("click", openFilm));
  $(".modal-close").addEventListener("click", closeFilm);
  modal.addEventListener("click", e => { if (e.target === modal) closeFilm(); });
  addEventListener("keydown", e => { if (e.key === "Escape" && !modal.hidden) closeFilm(); });
})();
