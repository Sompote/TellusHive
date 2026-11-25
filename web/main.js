// TellusHive web: the live beam calculation, scroll reveals, the result count and checksum copy.
(() => {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---- live calculation: a point load P on a simply supported beam of span L ----
  // Reactions RA = P b / L, RB = P a / L; maximum moment under the load M = P a b / L.
  // The section line is indicative: fy = 400 MPa, d = 540 mm, lever arm 0.9 d, phi 0.9, DB25 = 491 mm2.
  const P = 120, L = 8, X0 = 80, X1 = 920, Y0 = 262, MSCALE = 0.5; // px per kNm
  const stage = document.getElementById("stage");
  const load = document.getElementById("load");
  const bmd = document.getElementById("bmd");
  const mmax = document.getElementById("mmax");
  const mmaxT = document.getElementById("mmax-t");
  const raT = document.getElementById("ra-t");
  const rbT = document.getElementById("rb-t");
  const out = { a: document.getElementById("o-a"), ra: document.getElementById("o-ra"), rb: document.getElementById("o-rb"), m: document.getElementById("o-m"), bar: document.getElementById("o-bar") };
  const svg = document.getElementById("calc");

  let target = 4, cur = 4, pointer = false, t0 = performance.now(), running = false;

  const draw = (a) => {
    const b = L - a, ra = (P * b) / L, rb = (P * a) / L, m = (P * a * b) / L;
    const x = X0 + (a / L) * (X1 - X0), y = Y0 + m * MSCALE;
    load.setAttribute("transform", `translate(${x.toFixed(1)} 0)`);
    bmd.setAttribute("d", `M${X0} ${Y0} L${x.toFixed(1)} ${y.toFixed(1)} L${X1} ${Y0} Z`);
    mmax.setAttribute("x1", x.toFixed(1)); mmax.setAttribute("x2", x.toFixed(1)); mmax.setAttribute("y2", y.toFixed(1));
    mmaxT.setAttribute("x", Math.min(860, Math.max(140, x)).toFixed(1));
    mmaxT.setAttribute("y", (y + 26).toFixed(1));
    mmaxT.textContent = `${m.toFixed(1)} kNm`;
    raT.textContent = `${ra.toFixed(1)} kN`;
    rbT.textContent = `${rb.toFixed(1)} kN`;
    out.a.textContent = a.toFixed(2);
    out.ra.textContent = ra.toFixed(1);
    out.rb.textContent = rb.toFixed(1);
    out.m.textContent = m.toFixed(1);
    const as = (m * 1e6) / (0.9 * 400 * 0.9 * 540);
    out.bar.textContent = `${Math.max(2, Math.ceil(as / 491))} DB25`;
  };

  const place = (ev) => {
    const r = svg.getBoundingClientRect();
    const px = ((ev.clientX - r.left) / r.width) * 1000;
    target = Math.min(L - 0.25, Math.max(0.25, ((px - X0) / (X1 - X0)) * L));
    pointer = true;
    kick();
  };
  stage.addEventListener("pointermove", place);
  stage.addEventListener("pointerdown", place);
  stage.addEventListener("pointerleave", () => { pointer = false; t0 = performance.now(); kick(); });

  const frame = (now) => {
    if (!pointer && !reduce && now - t0 > 3000) {
      // idle: after the page settles, the load walks slowly along the span
      target = 4 + 2.6 * Math.sin((now - t0 - 3000) / 2600);
    }
    cur += (target - cur) * (reduce ? 1 : 0.14);
    draw(cur);
    if (reduce && Math.abs(target - cur) < 1e-3) { running = false; return; }
    requestAnimationFrame(frame);
  };
  const kick = () => { if (!running) { running = true; requestAnimationFrame(frame); } };
  draw(cur);
  kick();

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
