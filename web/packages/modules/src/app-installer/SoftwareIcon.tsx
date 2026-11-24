import React from "react";

// A tile of the panel's own: the first letters of the name on a colour taken from the name.
const COLOURS = ["#2563eb", "#7c3aed", "#db2777", "#ea580c", "#16a34a", "#0891b2", "#4f46e5", "#ca8a04"];

function hash(text: string): number {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h;
}

export function SoftwareIcon({ name, size = 44 }: { name: string; size?: number }) {
  const letters = name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  return (
    <div
      aria-hidden="true"
      className="flex shrink-0 items-center justify-center rounded-lg font-semibold text-white"
      style={{ width: size, height: size, background: COLOURS[hash(name) % COLOURS.length], fontSize: size / 2.6 }}
    >
      {letters || "?"}
    </div>
  );
}
