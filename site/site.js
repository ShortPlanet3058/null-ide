// Null's site: the nav that steps back, the first screen's code typing itself, words lit
// as you scroll, the features' picture that follows the words, the themes live, the
// platform you're on, and the trailer over the page.
(() => {
  "use strict";
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => [...root.querySelectorAll(s)];
  document.documentElement.classList.add("js");
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
  const play = video => { if (!still) video.play().catch(() => {}); };
  const load = video => { if (video.dataset.src && !video.src) video.src = video.dataset.src; };

  // ---------- the nav: solid once the page moves, away while you read on ----------
  const nav = $("#nav");
  let lastY = scrollY;
  function navOnScroll() {
    const y = scrollY;
    nav.classList.toggle("solid", y > 40);
    $("#scroll-hint").classList.toggle("gone", y > 40);
    if (Math.abs(y - lastY) > 6) {
      nav.classList.toggle("away", y > lastY && y > innerHeight * .7 && !nav.matches(":focus-within"));
      lastY = y;
    }
  }

  // ---------- the first screen: the name typed, then one word retyped, one caret ----------
  const nameText = $("#name-text"), nameCaret = $("#name-caret");
  const wordText = $("#word-text"), wordCaret = $("#word-caret"), wordBox = $(".word");
  const WORDS = ["calm", "fast", "precise", "quiet", "free"];
  const wait = ms => new Promise(r => setTimeout(r, ms));
  const caret = (el, state) => { el.classList.toggle("off", state === "off"); el.classList.toggle("solid", state === "solid"); };
  async function typeInto(el, text, base) {
    for (const ch of text) { el.textContent += ch; await wait(base + Math.random() * base * .8); }
  }
  async function firstScreen() {
    if (still) return;
    nameText.textContent = "";
    caret(nameCaret, "solid");
    await wait(700);
    await typeInto(nameText, "Null", 120);
    caret(nameCaret, "blink");
    await wait(2600);
    // The caret leaves the name for the word, and changes it the way you would.
    for (let i = 1; ; i = (i + 1) % WORDS.length) {
      caret(nameCaret, "off");
      caret(wordCaret, "blink");
      await wait(900);
      wordBox.classList.add("sel");
      await wait(520);
      wordBox.classList.remove("sel");
      wordText.textContent = "";
      caret(wordCaret, "solid");
      await wait(160);
      await typeInto(wordText, WORDS[i], 70);
      caret(wordCaret, "blink");
      await wait(2800);
      while (document.hidden) await wait(500);
    }
  }
  firstScreen();

  // ---------- the platform you're on ----------
  const ua = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent;
  const phone = /iphone|ipad|android/i.test(navigator.userAgent);
  const os = phone ? null : /mac/i.test(ua) ? "mac" : /win/i.test(ua) ? "windows" : /linux/i.test(ua) ? "linux" : null;
  if (os) {
    const mine = $(`.platform[data-os="${os}"]`);
    mine.classList.add("mine");
    $("#hero-download-label").textContent = `Download for ${{ mac: "Mac", windows: "Windows", linux: "Linux" }[os]}`;
    $("#hero-download").href = $("a.button", mine).href;
  }
  // Keys as this computer names them.
  $$(".keys .k").forEach(k => {
    k.innerHTML = (os === "mac" || (!os && /mac|iphone|ipad/i.test(navigator.userAgent)) ? k.dataset.mac : k.dataset.pc)
      .split(" ").map(key => `<kbd>${key}</kbd>`).join("");
  });

  // ---------- words lit as you scroll ----------
  const words = $("#words");
  const KEY = new Set(["your", "code,"]);
  words.innerHTML = words.textContent.split(/(\s+)/)
    .map(w => (/\s/.test(w) ? w : `<span${KEY.has(w) ? ' class="key"' : ""}>${w}</span>`)).join("");
  const wordSpans = $$("span", words);
  function lightWords() {
    const r = words.getBoundingClientRect();
    if (r.bottom < -100 || r.top > innerHeight + 100) return;
    const p = still ? 1 : clamp((innerHeight * .78 - r.top) / (r.height + innerHeight * .2));
    const lit = Math.round(p * wordSpans.length);
    wordSpans.forEach((w, i) => w.classList.toggle("lit", i < lit));
  }

  // ---------- what it does: the picture follows the words ----------
  const steps = $$(".step"), screen = $("#screen"), ticks = $("#ticks");
  const shown = steps.map(step => {
    const v = document.createElement("video");
    Object.assign(v, { muted: true, loop: true, playsInline: true, preload: "none" });
    v.setAttribute("muted", ""); v.setAttribute("playsinline", "");
    const own = $(".step-media video", step);
    v.poster = own.getAttribute("poster");
    v.dataset.src = own.dataset.src;
    screen.appendChild(v);
    ticks.appendChild(document.createElement("i"));
    return v;
  });
  const tickBars = $$("i", ticks);
  const wide = matchMedia("(min-width: 961px)");
  let active = -1;
  function follow() {
    if (!wide.matches) return;
    const box = screen.getBoundingClientRect();
    if (box.bottom < -200 || box.top > innerHeight + 200) {
      if (active >= 0) shown[active].pause();
      return;
    }
    const mid = innerHeight / 2;
    let on = 0;
    steps.forEach((s, i) => { if (s.getBoundingClientRect().top < mid) on = i; });
    const r = steps[on].getBoundingClientRect();
    const within = clamp((mid - r.top) / r.height);
    tickBars.forEach((t, i) => t.style.setProperty("--f", i < on ? 1 : i === on ? within.toFixed(3) : 0));
    if (on !== active) {
      steps.forEach((s, i) => s.classList.toggle("on", i === on));
      shown.forEach((v, i) => {
        if (i === on) { load(v); v.classList.add("on"); v.currentTime = 0; play(v); }
        else { v.classList.remove("on"); setTimeout(() => { if (!v.classList.contains("on")) v.pause(); }, 800); }
      });
      // The next one, ready before it's needed.
      if (shown[on + 1]) { load(shown[on + 1]); shown[on + 1].preload = "auto"; }
      active = on;
    } else if (shown[on].paused) play(shown[on]);
  }

  // ---------- videos that play while they're seen (the phone's features, AI) ----------
  const seenVideos = new IntersectionObserver(entries => entries.forEach(e => {
    const v = e.target;
    if (e.isIntersecting) { load(v); play(v); } else v.pause();
  }), { rootMargin: "120px 0px", threshold: .25 });
  $$(".step-media video, video[data-inview]").forEach(v => seenVideos.observe(v));

  // ---------- the themes, live ----------
  const THEMES = {
    Null: { bg: "#000000", surface: "#000000", fg: "#ebe8e2", muted: "#8b8a92", faint: "#46464e", line: "#1d1d22", raised: "#0f0f12", caret: "#f2b35b", kw: "#c6a6f6", ty: "#87c7e1", fn: "#f1d9a9", st: "#a9d59c", nu: "#f3a37e", co: "#5f626e", pu: "#8d8c96" },
    Ash: { bg: "#131417", surface: "#18191d", fg: "#e4e3e0", muted: "#8f8f98", faint: "#4d4e56", line: "#26272d", raised: "#212227", caret: "#efb565", kw: "#c4a8f0", ty: "#8ac6dd", fn: "#eed9b0", st: "#acd3a0", nu: "#f0a685", co: "#676a75", pu: "#91909a" },
    Midnight: { bg: "#0b0e14", surface: "#0f131b", fg: "#dfe4ee", muted: "#8590a6", faint: "#414b60", line: "#1c2230", raised: "#151a24", caret: "#7cc4e8", kw: "#b7a3f2", ty: "#8bd0e6", fn: "#e9d7a8", st: "#a4d4a3", nu: "#f0a682", co: "#58627a", pu: "#8890a3" },
    Moss: { bg: "#101311", surface: "#141815", fg: "#e2e5dd", muted: "#8c9488", faint: "#474f47", line: "#222823", raised: "#1a1f1b", caret: "#b5cf85", kw: "#d0a6d8", ty: "#8ec9c0", fn: "#e8d39c", st: "#b7d48f", nu: "#eba37c", co: "#606b5f", pu: "#8f968b" },
    "High Contrast": { bg: "#000000", surface: "#000000", fg: "#ffffff", muted: "#c8c8d0", faint: "#8e8e98", line: "#5a5a64", raised: "#0c0c0f", caret: "#ffbe55", kw: "#d8bcff", ty: "#8edcff", fn: "#ffe2a8", st: "#b4f09f", nu: "#ffad85", co: "#a8acba", pu: "#d0d0d8" },
    Paper: { bg: "#fcfcfb", surface: "#f4f4f2", fg: "#1d1e22", muted: "#6a6b72", faint: "#b0b1b7", line: "#e6e5e1", raised: "#ffffff", caret: "#b4620c", kw: "#7a3ec8", ty: "#1e6c92", fn: "#8c4a00", st: "#3a7a2e", nu: "#b4461e", co: "#9a9ca3", pu: "#6f7077" },
    Dune: { bg: "#f6f1e7", surface: "#efe8da", fg: "#2b2620", muted: "#75695a", faint: "#b6aa97", line: "#e2d9c7", raised: "#fbf8f1", caret: "#b5532d", kw: "#8a3f8f", ty: "#2c6e7f", fn: "#8f4a10", st: "#527a26", nu: "#b5532d", co: "#a2967f", pu: "#7a6e5e" },
  };
  const names = Object.keys(THEMES), themeWindow = $("#theme-window"), picker = $("#theme-picker");
  let theme = 0, chosen = false, themeSeen = false;
  const swatches = names.map((name, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "radio");
    b.innerHTML = `<span class="sw" style="background:${THEMES[name].bg};--c:${THEMES[name].caret}"></span>${name}`;
    b.addEventListener("click", () => { chosen = true; applyTheme(i); });
    picker.appendChild(b);
    return b;
  });
  function applyTheme(i) {
    theme = i;
    const th = THEMES[names[i]];
    for (const [k, v] of Object.entries(th)) themeWindow.style.setProperty(`--${k}`, v);
    swatches.forEach((b, k) => b.setAttribute("aria-checked", k === i));
  }
  applyTheme(0);
  // They take turns by themselves while seen, until you pick one.
  new IntersectionObserver(([e]) => { themeSeen = e.isIntersecting; }, { threshold: .5 }).observe(themeWindow);
  if (!still) setInterval(() => { if (themeSeen && !chosen && !document.hidden) applyTheme((theme + 1) % names.length); }, 2600);

  // ---------- things arriving as they come into view ----------
  const arrive = new IntersectionObserver(entries => entries.forEach(e => {
    if (!e.isIntersecting) return;
    e.target.classList.add("in");
    arrive.unobserve(e.target);
  }), { threshold: .12, rootMargin: "0px 0px -6% 0px" });
  $$(".platforms .reveal").forEach((el, i) => { el.style.transitionDelay = `${i * 110}ms`; });
  $$(".reveal").forEach(el => arrive.observe(el));

  // ---------- one loop for what follows the scroll ----------
  let queued = false;
  function frame() {
    queued = false;
    navOnScroll();
    lightWords();
    follow();
  }
  const schedule = () => { if (!queued) { queued = true; requestAnimationFrame(frame); } };
  addEventListener("scroll", schedule, { passive: true });
  addEventListener("resize", schedule);
  wide.addEventListener("change", () => { active = -1; schedule(); });
  frame();

  // ---------- the trailer, over the page ----------
  const modal = $("#modal"), modalVideo = $("#modal-video");
  let opener = null;
  function openTrailer(e) {
    opener = e.currentTarget;
    modal.hidden = false;
    document.body.style.overflow = "hidden";
    requestAnimationFrame(() => modal.classList.add("open"));
    modalVideo.play().catch(() => {});
    $(".modal-close").focus();
  }
  function closeTrailer() {
    modal.classList.remove("open");
    modalVideo.pause();
    document.body.style.overflow = "";
    setTimeout(() => { modal.hidden = true; }, 450);
    if (opener) opener.focus();
  }
  $$("[data-trailer]").forEach(b => b.addEventListener("click", openTrailer));
  $(".modal-close").addEventListener("click", closeTrailer);
  modal.addEventListener("click", e => { if (e.target === modal) closeTrailer(); });
  addEventListener("keydown", e => { if (e.key === "Escape" && !modal.hidden) closeTrailer(); });
})();
