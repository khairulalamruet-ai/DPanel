/**
 * Some reads go to a node and answer `202 { operation_id }` when the node is slow: the same
 * request is simply asked again a moment later. This asks until there is an answer.
 */
export async function untilAnswered<T>(
  call: () => Promise<T>,
  opts: { intervalMs?: number; tries?: number } = {},
): Promise<T> {
  const { intervalMs = 1000, tries = 30 } = opts;
  for (let i = 0; i < tries; i++) {
    const res = await call();
    const pending = res !== null && typeof res === "object" && "operation_id" in (res as object);
    if (!pending) return res;
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error("The server is taking too long to answer; try again in a moment");
}
