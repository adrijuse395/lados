import fs from "node:fs";

const source = fs.readFileSync(new URL("./lados-mark-source-trace.svg", import.meta.url), "utf8");
const full = source.match(/fill="rgb\(1,1,1\)" d="([^"]+)"/)?.[1] ?? source.match(/d="([^"]+)"/)[1];
const parts = full.split(/(?<=Z)(?=M)/);
const dragon = parts.slice(2).join("");
const shield = parts.slice(0, 2).join("");

function sample(d) {
  const tokens = d.match(/[MLCZ]|-?\d*\.?\d+/g);
  let i = 0;
  let x = 0;
  let y = 0;
  let sx = 0;
  let sy = 0;
  const pts = [];
  while (i < tokens.length) {
    const cmd = tokens[i++];
    if (cmd === "M" || cmd === "L") {
      x = Number(tokens[i++]);
      y = Number(tokens[i++]);
      if (cmd === "M") {
        sx = x;
        sy = y;
      }
      pts.push([x, y]);
    } else if (cmd === "C") {
      const x1 = Number(tokens[i++]);
      const y1 = Number(tokens[i++]);
      const x2 = Number(tokens[i++]);
      const y2 = Number(tokens[i++]);
      const x3 = Number(tokens[i++]);
      const y3 = Number(tokens[i++]);
      for (let step = 1; step <= 16; step++) {
        const u = step / 16;
        const a = 1 - u;
        pts.push([
          a * a * a * x + 3 * a * a * u * x1 + 3 * a * u * u * x2 + u * u * u * x3,
          a * a * a * y + 3 * a * a * u * y1 + 3 * a * u * u * y2 + u * u * u * y3,
        ]);
      }
      x = x3;
      y = y3;
    } else if (cmd === "Z") {
      x = sx;
      y = sy;
    }
  }
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const minX = Math.min(...xs);
  const minY = Math.min(...ys);
  return { minX, minY, w: Math.max(...xs) - minX, h: Math.max(...ys) - minY };
}

function placed(d, { fill, pad }) {
  const box = sample(d);
  const size = 1024;
  const scale = (size * (1 - pad * 2)) / Math.max(box.w, box.h);
  const tx = (size - box.w * scale) / 2 - box.minX * scale;
  const ty = (size - box.h * scale) / 2 - box.minY * scale;
  return { scale, tx, ty, fill, d };
}

function svg({ plate, fill, rx = 0, piece, pad }) {
  const p = placed(piece, { fill, pad });
  const bg = plate ? `<rect width="1024" height="1024" rx="${rx}" fill="${plate}"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-label="Lados">${bg}<g transform="translate(${p.tx.toFixed(2)} ${p.ty.toFixed(2)}) scale(${p.scale.toFixed(5)})"><path fill="${fill}" d="${piece}"/></g></svg>\n`;
}

const files = {
  "lados-mark-master.svg": svg({ piece: full, fill: "#111418", pad: 0.04 }),
  "lados-mark-on-light.svg": svg({ piece: full, fill: "#111418", pad: 0.04 }),
  "lados-mark-on-dark.svg": svg({ piece: full, fill: "#F4F5F7", pad: 0.04 }),
  "lados-mark-current.svg": svg({ piece: full, fill: "currentColor", pad: 0.04 }),
  "lados-mark-dragon.svg": svg({ piece: dragon, fill: "#111418", pad: 0.08 }),
  "lados-mark-dragon-on-dark.svg": svg({ piece: dragon, fill: "#F4F5F7", pad: 0.08 }),
  "lados-mark-dragon-current.svg": svg({ piece: dragon, fill: "currentColor", pad: 0.08 }),
  "lados-mark-shield.svg": svg({ piece: shield, fill: "#111418", pad: 0.08 }),
  "lados-app-icon.svg": svg({ piece: full, fill: "#F4F5F7", plate: "#111418", rx: 176, pad: 0.055 }),
  "lados-app-icon-light.svg": svg({ piece: full, fill: "#111418", plate: "#F4F5F7", rx: 176, pad: 0.055 }),
  "lados-app-icon-dragon.svg": svg({ piece: dragon, fill: "#F4F5F7", plate: "#111418", rx: 176, pad: 0.07 }),
  "lados-favicon.svg": svg({ piece: dragon, fill: "#F4F5F7", plate: "#111418", rx: 224, pad: 0.14 }),
};

for (const [name, body] of Object.entries(files)) fs.writeFileSync(name, body);

function lockup(horizontal, onDark) {
  const ink = onDark ? "#F4F5F7" : "#111418";
  const mark = placed(full, { fill: ink, pad: 0 });
  if (horizontal) {
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1680 520" role="img" aria-label="Lados"><g transform="translate(36 36) scale(0.438)"><g transform="translate(${mark.tx.toFixed(2)} ${mark.ty.toFixed(2)}) scale(${mark.scale.toFixed(5)})"><path fill="${ink}" d="${full}"/></g></g><text x="560" y="328" fill="${ink}" font-family="Segoe UI, system-ui, sans-serif" font-size="210" font-weight="650" letter-spacing="-4">Lados</text></svg>\n`;
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1240" role="img" aria-label="Lados"><g transform="translate(150 24) scale(0.708)"><g transform="translate(${mark.tx.toFixed(2)} ${mark.ty.toFixed(2)}) scale(${mark.scale.toFixed(5)})"><path fill="${ink}" d="${full}"/></g></g><text x="512" y="1148" text-anchor="middle" fill="${ink}" font-family="Segoe UI, system-ui, sans-serif" font-size="156" font-weight="650" letter-spacing="-2">Lados</text></svg>\n`;
}

fs.writeFileSync("lados-wordmark-horizontal.svg", lockup(true, false));
fs.writeFileSync("lados-wordmark-horizontal-on-dark.svg", lockup(true, true));
fs.writeFileSync("lados-wordmark-stacked.svg", lockup(false, false));
fs.copyFileSync("lados-app-icon-dragon.svg", "../app-icon.svg");

const mark = placed(full, { fill: "#F4F5F7", pad: 0.055 });
const head = placed(dragon, { fill: "currentColor", pad: 0.08 });
const icons = new URL("../src/ui/icons.ts", import.meta.url);
let ts = fs.readFileSync(icons, "utf8");
const start = ts.indexOf("const LADOS_MARK");
if (start < 0) throw new Error("logo block missing");
const block = `const LADOS_MARK = ${JSON.stringify(full)};
const LADOS_DRAGON = ${JSON.stringify(dragon)};

/** Full mark on the dark plate. Used large, on the welcome and unlock screens. */
export function logo(className = "logo"): SVGSVGElement {
  return mark(className, LADOS_MARK, ${mark.tx.toFixed(2)}, ${mark.ty.toFixed(2)}, ${mark.scale.toFixed(5)}, true);
}

/** Dragon head only, in the current text color. Used where the full shield would be too small. */
export function dragonMark(className = "logo"): SVGSVGElement {
  return mark(className, LADOS_DRAGON, ${head.tx.toFixed(2)}, ${head.ty.toFixed(2)}, ${head.scale.toFixed(5)}, false);
}

function mark(className: string, path: string, x: number, y: number, scale: number, plate: boolean): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 1024 1024");
  svg.setAttribute("class", className);
  svg.setAttribute("aria-hidden", "true");
  const background = plate ? '<rect width="1024" height="1024" rx="176" fill="#111418"/>' : "";
  const ink = plate ? "#f4f5f7" : "currentColor";
  svg.innerHTML = \`\${background}<g transform="translate(\${x} \${y}) scale(\${scale})"><path fill="\${ink}" d="\${path}"/></g>\`;
  return svg;
}
`;
fs.writeFileSync(icons, ts.slice(0, start) + block);
console.log("wrote marks", parts.length);
