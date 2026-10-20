import {loadRecipes, selectPackages, options, main} from './lib.mjs';

main(async () => {
  const args = options(process.argv.slice(2));
  if (args.help) { console.log('node scripts/language-packs/check.mjs [--all | --packages rust,python] [--recipes file]'); return; }
  const recipes = loadRecipes(args.recipes), selected = selectPackages(recipes,args);
  console.log(`Validated ${Object.keys(recipes.packages).length} recipes; selected ${selected.length}: ${selected.join(', ')}`);
});
