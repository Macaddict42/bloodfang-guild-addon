const assert = require("assert");
const {
  partitionUploadLines,
  humanizeUploadLines,
  refusalMessage,
  uploadFailureDropsToken,
} = require("./upload-lines.js");

const mixed = partitionUploadLines([
  "gbank site_refused",
  "recipes accepted",
]);
assert.deepStrictEqual(mixed.refused, [{ ty: "gbank", error: "site_refused" }]);
assert.deepStrictEqual(mixed.accepted, ["recipes accepted"]);
assert.strictEqual(humanizeUploadLines(["gbank site_refused", "recipes accepted"]), "profession recipes");

const bank = refusalMessage(mixed.refused);
assert.strictEqual(bank.dropsToken, false);
assert.strictEqual(bank.title, "Couldn't send guild bank");
assert.match(bank.message, /guild bank snapshot: site_refused/);
assert.match(bank.message, /still saved/);
assert.doesNotMatch(bank.message, /Officer/);
assert.doesNotMatch(bank.message, /profession recipes/);

const own = refusalMessage([{ ty: "recipes", error: "site_refused" }]);
assert.strictEqual(own.title, "Some strings were not accepted");
assert.match(own.message, /profession recipes: site_refused/);
assert.doesNotMatch(own.message, /Officer/);
assert.strictEqual(own.dropsToken, false);

assert.deepStrictEqual(partitionUploadLines(["gbank duplicate"]).refused, []);
assert.strictEqual(uploadFailureDropsToken("the site rejected the uploader token; make a new one"), true);
assert.strictEqual(uploadFailureDropsToken("guild bank snapshot: site_refused. This sign-in is still saved."), false);
assert.strictEqual(uploadFailureDropsToken("Sign in again with a Discord account that has the right role."), false);

console.log("upload-lines.test.js ok");
