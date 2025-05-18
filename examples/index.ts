import { isPool } from './ts-suigen/champ_market/cpmm';
import { champ_market } from './ts-suigen';
import { mockcoins } from './ts-suigen';

const poolType = champ_market.cpmm.Pool.r(mockcoins.red.RED.phantom(), mockcoins.blue.BLUE.phantom());
console.log(poolType.fullTypeName);

console.log(isPool(poolType.fullTypeName));
