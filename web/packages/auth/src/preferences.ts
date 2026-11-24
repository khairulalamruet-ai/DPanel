import { apiFetch } from "@builtpanel/api";

/**
 * Keeps the chosen language in the account's preferences (the saved favourites stay as they are),
 * so the next device opens in it. A failure is ignored: the browser still remembers the choice.
 */
export async function saveLocalePreference(locale: string): Promise<void> {
  try {
    const current = await apiFetch<{ favorites: string[]; locale?: string | null }>("/api/v1/auth/preferences");
    await apiFetch("/api/v1/auth/preferences", {
      method: "PUT",
      body: JSON.stringify({ favorites: current.favorites ?? [], locale }),
    });
  } catch {
    // Not signed in, or offline.
  }
}
