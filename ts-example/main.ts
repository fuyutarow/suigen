import { champ_market, mockcoins } from "./src/suigen";
import { isPool } from "./src/suigen/champ_market/cpmm";

const poolType = champ_market.cpmm.Pool.r(
	mockcoins.red.RED.phantom(),
	mockcoins.blue.BLUE.phantom(),
);
console.log(poolType.fullTypeName);

console.log(isPool(poolType.fullTypeName));
