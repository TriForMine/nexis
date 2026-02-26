export async function initializeBetterAuth(): Promise<void> {
  try {
    await import("better-auth");
  } catch (error) {
    console.warn("BetterAuth initialization skipped", error);
  }
}
