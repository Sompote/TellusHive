// TellusHive web: the team at work, scroll reveals, the result count and checksum copy.
(() => {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---- the team at work: one request through the design office (an illustrative run) ----
  // Each step changes some nodes (state text, progress bar, active or done), some links and
  // the output counts; the run replays while the figure is on screen.
  const crew = document.querySelector(".crew");
  if (crew) {
    const $ = (id) => document.getElementById(id);
    const NODES = ["arun", "coord", "designer", "geo", "checker", "drafter", "reporter"];
    const LINKS = ["arun", "coord", "designer", "geo", "checker", "drafter", "reporter", "check"];
    const STEPS = [
      [0, "The engineer sends one message.", {}, {}, {}],
      [800, "Arun, the bot, reads the request.", { arun: ["active", "Reading"] }, { arun: "active" }, {}],
      [2200, "Arun hands the job to the design office.", { arun: ["active", "Delegating"] }, { arun: "done", coord: "active" }, {}],
      [3200, "The coordinator splits the work.", { arun: ["", "Waiting"], coord: ["active", "Planning 3 tasks"] }, { coord: "done" }, {}],
      [4400, "The designer and the geotechnical analyst search the standards.", { coord: ["active", "Delegating"], designer: ["active", "Searching", 30], geo: ["active", "Searching", 25] }, { designer: "active", geo: "active" }, {}],
      [6000, "They calculate to the clauses they found.", { designer: ["active", "Calculating", 70], geo: ["active", "Calculating", 60] }, {}, {}],
      [7600, "Two calculation sheets saved, each with its clause references.", { designer: ["done", "Saved \u2713", 100], geo: ["done", "Saved \u2713", 100] }, { designer: "done", geo: "done" }, { calc: 2 }],
      [8400, "The checker recomputes both results independently.", { coord: ["active", "Checking"], checker: ["active", "Recomputing", 50] }, { checker: "active", check: "active" }, {}],
      [10000, "Both checks pass.", { checker: ["done", "PASS \u2713", 100] }, { checker: "done", check: "done" }, { check: 2 }],
      [10800, "The drafter draws the pile layout and the cap.", { drafter: ["active", "Drawing", 50] }, { drafter: "active" }, {}],
      [12200, "The reporter writes the calculation report.", { drafter: ["done", "Done \u2713", 100], reporter: ["active", "Writing", 50] }, { drafter: "done", reporter: "active" }, { draw: 2 }],
      [13800, "Arun reports back, citing every clause it used.", { reporter: ["done", "Done \u2713", 100], coord: ["done", "Done \u2713"], arun: ["active", "Reporting back"] }, { reporter: "done" }, { rep: 1 }],
      [15400, "Done: every number traces back to its sheet.", { arun: ["done", "Done \u2713"] }, {}, {}],
    ];
    const LOOP = 19000;
    const now = $("o-now");
    const reset = () => {
      NODES.forEach((n) => {
        const g = $("n-" + n); g.classList.remove("is-active", "is-done");
        const st = $("s-" + n); if (st) st.textContent = n === "arun" ? "Idle" : "Waiting";
        const b = $("b-" + n); if (b) b.setAttribute("width", "0");
      });
      LINKS.forEach((l) => $("l-" + l).classList.remove("is-active", "is-done"));
      ["calc", "check", "draw", "rep"].forEach((k) => { $("o-" + k).textContent = "0"; });
    };
    const apply = ([, text, nodes, links, outs]) => {
      now.textContent = text;
      Object.entries(nodes).forEach(([n, [state, label, pct]]) => {
        const g = $("n-" + n);
        g.classList.toggle("is-active", state === "active");
        g.classList.toggle("is-done", state === "done");
        $("s-" + n).textContent = label;
        if (pct !== undefined) $("b-" + n).setAttribute("width", String(1.52 * pct));
      });
      Object.entries(links).forEach(([l, state]) => {
        const el = $("l-" + l);
        el.classList.toggle("is-active", state === "active");
        el.classList.toggle("is-done", state === "done");
      });
      Object.entries(outs).forEach(([k, v]) => { $("o-" + k).textContent = String(v); });
    };
    if (reduce) {
      // no motion: the finished run, in one look
      reset(); STEPS.forEach(apply);
      LINKS.forEach((l) => $("l-" + l).classList.replace("is-active", "is-done"));
    } else {
      // each step on its own timer, the whole run again after LOOP, only while it is on screen
      let timers = [], visible = false;
      const play = () => {
        reset();
        timers = STEPS.map((st) => setTimeout(() => apply(st), st[0]));
        timers.push(setTimeout(play, LOOP));
      };
      const run = () => { if (visible && !document.hidden && !timers.length) play(); };
      const pause = () => { timers.forEach(clearTimeout); timers = []; reset(); };
      reset();
      if ("IntersectionObserver" in window) {
        new IntersectionObserver(([e]) => { visible = e.isIntersecting; visible ? run() : pause(); }, { threshold: 0.25 }).observe(crew);
      } else { visible = true; run(); }
      document.addEventListener("visibilitychange", () => (document.hidden ? pause() : run()));
    }
  }

  // ---- scroll reveal ----
  const els = document.querySelectorAll(".statement__text, .notes li, .team__head, .reg, .evidence__figure, .evidence__grid, .flow .h2, .tick, .download > *");
  if (!reduce && "IntersectionObserver" in window) {
    els.forEach((el, i) => { el.classList.add("reveal"); el.style.transitionDelay = `${(i % 6) * 60}ms`; });
    const io = new IntersectionObserver((entries) => {
      entries.forEach((e) => { if (e.isIntersecting) { e.target.classList.add("is-in"); io.unobserve(e.target); } });
    }, { rootMargin: "0px 0px -10% 0px" });
    els.forEach((el) => io.observe(el));
  }

  // ---- the result counts up once, from the control's 26 to 35 ----
  const count = document.getElementById("count");
  if (count && !reduce && "IntersectionObserver" in window) {
    const to = Number(count.dataset.to), from = 26;
    count.textContent = String(from);
    const io = new IntersectionObserver(([e]) => {
      if (!e.isIntersecting) return;
      io.disconnect();
      const start = performance.now(), dur = 1200;
      const step = (now) => {
        const p = Math.min(1, (now - start) / dur), eased = 1 - Math.pow(1 - p, 3);
        count.textContent = String(Math.round(from + (to - from) * eased));
        if (p < 1) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }, { threshold: 0.6 });
    io.observe(count);
  }

  // ---- copy a checksum ----
  document.querySelectorAll(".copy").forEach((btn) => {
    const label = btn.textContent;
    btn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(btn.dataset.sum);
        btn.dataset.state = "done";
        btn.textContent = "Copied";
      } catch {
        btn.textContent = btn.dataset.sum;
      }
      setTimeout(() => { btn.dataset.state = ""; btn.textContent = label; }, 1600);
    });
  });
})();
