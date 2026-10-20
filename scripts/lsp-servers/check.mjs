// Offline checks: the recipes are well formed, every download is pinned and every npm
// pack has a lockfile. Needs no network and builds nothing.
import {loadRecipes, loadPins, validatePins, validateLocks, validateCatalog, readToml, CATALOG, main} from './lib.mjs';

main(async () => {
  const recipes = loadRecipes(), pins = loadPins();
  validatePins(recipes, pins); validateLocks(recipes);
  validateCatalog(readToml(CATALOG));
  console.log(`${Object.keys(recipes.packs).length} packs, ${Object.keys(recipes.servers).length} servers: recipes, pins and lockfiles agree`);
});
