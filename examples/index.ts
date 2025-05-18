import { isPool } from './ts-suigen/champ_market/cpmm';

console.log(
  isPool(
    '0x41f17137266d55fe4a1c954e081fe12505a846313fae514c5064abd5e6c7181d::cpmm::Pool<0x2::sui::SUI, 0x2::sui::SUI>',
  ),
);
