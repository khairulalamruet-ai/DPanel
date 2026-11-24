import React, { useState } from "react";

// Official application logo map from dc2 softaculous/webuzo images
const LOGO_MAP: Record<string, string> = {
  redis: "/apps/redis.png",
  postgresql: "/apps/postgresql.gif",
  postgres: "/apps/postgresql.gif",
  pgsql: "/apps/postgresql.gif",
  nginx: "/apps/nginx.gif",
  php: "/apps/php.gif",
  apache: "/apps/apache.gif",
  apache2: "/apps/apache.gif",
  mariadb: "/apps/mariadb.gif",
  mysql: "/apps/mariadb.gif",
  nodejs: "/apps/nodejs.jpg",
  node: "/apps/nodejs.jpg",
  python: "/apps/python.gif",
  python3: "/apps/python.gif",
  mongodb: "/apps/mongodb.gif",
  mongo: "/apps/mongodb.gif",
  phpmyadmin: "/apps/phpmyadmin.gif",
  wordpress: "/apps/wordpress.png",
  wp: "/apps/wordpress.png",
  laravel: "/apps/laravel.png",
  joomla: "/apps/joomla.png",
  drupal: "/apps/drupal.png",
  static: "/apps/static.png",
  roundcube: "/apps/roundcube.png",
  whmcs: "/apps/whmcs.png",
  prestashop: "/apps/prestashop.png",
  moodle: "/apps/moodle.png",
};

// Fallback tile: the first letters of the name on a colour taken from the name.
const COLOURS = ["#2563eb", "#7c3aed", "#db2777", "#ea580c", "#16a34a", "#0891b2", "#4f46e5", "#ca8a04"];

function hash(text: string): number {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h;
}

export interface SoftwareIconProps {
  name: string;
  slug?: string;
  size?: number;
  className?: string;
}

export function SoftwareIcon({ name, slug, size = 44, className }: SoftwareIconProps) {
  const [imgError, setImgError] = useState(false);

  const key = (slug || name).toLowerCase().replace(/[^a-z0-9]/g, "");
  const logoSrc = LOGO_MAP[slug?.toLowerCase() ?? ""] || LOGO_MAP[key] || LOGO_MAP[name.toLowerCase()];

  if (logoSrc && !imgError) {
    return (
      <div
        aria-hidden="true"
        className={`app-logo-box shrink-0 flex items-center justify-center rounded-xl bg-white dark:bg-card/90 border border-slate-200 dark:border-slate-800 p-2 shadow-xs overflow-hidden ${className ?? ""}`}
        style={{ width: size, height: size }}
      >
        <img
          src={logoSrc}
          alt={name}
          className="max-w-full max-h-full object-contain pointer-events-none select-none"
          onError={() => setImgError(true)}
        />
      </div>
    );
  }

  const letters = name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");

  return (
    <div
      aria-hidden="true"
      className={`flex shrink-0 items-center justify-center rounded-xl font-bold text-white shadow-xs ${className ?? ""}`}
      style={{
        width: size,
        height: size,
        background: COLOURS[hash(name) % COLOURS.length],
        fontSize: Math.max(12, Math.round(size / 2.6)),
      }}
    >
      {letters || "?"}
    </div>
  );
}
