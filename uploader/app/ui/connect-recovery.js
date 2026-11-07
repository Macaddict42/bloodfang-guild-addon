/* Waiting keeps the Cancel step. After cancel that button is gone, so the hint
   must not tell the user to press it. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.connectRecovery = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  const WAITING = "If the site says this account can't sign in, this app has not received that. Cancel here first, switch or log out of that Discord account in the browser, then use Sign in with a different account. That button does not open Discord's account list by itself. Sign out on the site only clears the site session.";
  const CANCELLED = "Sign-in was cancelled. To use another account, switch accounts or log out of Discord in your browser, then choose “Sign in with a different account” below. Signing out of the guild website does not switch Discord accounts.";

  function connectPhase(phase) {
    if (phase === "waiting") {
      return {
        recovery: WAITING,
        status: "Waiting for the browser.",
        cancelHidden: false,
        switchHidden: true,
      };
    }
    if (phase === "cancelled") {
      return {
        recovery: CANCELLED,
        status: "Sign-in didn't finish.",
        saved: "Nothing was saved.",
        cancelHidden: true,
        switchHidden: false,
        cta: "Sign in with a different account",
      };
    }
    return { recovery: "", cancelHidden: true, switchHidden: true };
  }

  return { connectPhase };
});
