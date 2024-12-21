TRUNCATE TABLE cs_configuration_v1;

DROP TABLE IF EXISTS users;
CREATE TABLE users (
    id bigint GENERATED ALWAYS AS IDENTITY,
    name text,
    email jsonb,
    PRIMARY KEY(id)
);

SELECT cs_add_index_v1(
  'users',
  'email',
  'unique',
  'text'
);

SELECT cs_encrypt_v1();
SELECT cs_activate_v1();

INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (1, 'user_1', '{"c": "mBbL(37yI+46`6ZLqKPh(mox;C$ADXs4S6lr~#OlkVL?}N1dw~|K^(S2<-(Nux%Ew4d+U9O5PQT#2~$^W}X4baFjie!-cQpY9PaFaL$hHbS~y;-pitWHUp<3Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (2, 'user_2', '{"c": "mBbKR1UbT#q!_ph+W`b3)nFCGFQqbkN>mHHhdgwzN&7jPX@Z5}i<}dQ=T=0|zKy~UFw#|l0Cg-GG#>AtRdb+R#2`&Q@b<ojxk9iENGtvj7goFi)$ZYV%Co^ocuKw2%(JH=Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (3, 'user_3', '{"c": "mBbJbz%WS$mr0E(=|$1|j8R9#FBv8<%PRTKYf}m*8ys12k{OhoQ1E{yYY69MW0o&fTSGkQ#ydI7&>9;{C_ctw#2{WYjBdR3_fHH<g;ly-ZgN!3DmeYVrRD!Dsc#*raeSvDWo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (4, 'user_4', '{"c": "mBbK)-hnlAb4Wl{F#j<!xD8pvFTTU{K!}}`@epO!8jt?GRipU)0lad;CNGsuP<U5YE_fisIZxs0UZ{T^$?N7i#31rO`(3>*uTHB0=6;w*d&=~ZFXK%Z*)-=*w+HFo+fAn;Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (5, 'user_5', '{"c": "mBbJs@kqo4<11mf=xg<z2+u3TFL`?F)ZKC7*1a1^8$6(Z_JTCJb8`GF4b&X<TSi7&YH8+Wv_2!>F}qo)8DVto#30`QgSa7AhO1p89{1vHw_EAAEr{Q5=GETs_zBUerwOMbWo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (6, 'user_6', '{"c": "mBbKp7QaPQeLcG`a$Ukt%nz@`FVWU;fUs7UEJIQhL*7)DL(5qEI}{MmVOLkrQ#^9LkpBL?-f6VgSnyb^ZUz}>#324k;Nu@>@fRyz=+(mYbBMSm{qC$@wp4T7Aw`!GNkFF}Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (7, 'user_7', '{"c": "mBbL)!TBKMd*9Bw_cneHa^k(jFBoLp>JQQHdd9Yn$E2e`>&b@<CH8J?0_*i9r|K1J`kLcVN(p!Ka9Rkj=5x!1#2`xXaI@65rg{4&PLKQBcNE5R=<znjS4&r}M?X**<n^Z_Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (8, 'user_8', '{"c": "mBbK&-tB=q{+7cot)7kD>fn{cFJmL($^a+sc>oR79?6FZi~%!vaiBB$+t6U%q)kkBhq88h7s|33T^rhDmwRc*#2~=0OlBR@hw>G`(qEW!%3de>G88k>#5=@=igd~Y0F<X9Wo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (9, 'user_9', '{"c": "mBbLyHA^AFRCJ~E41Qeq(WMx~FTGQmnLe}J?+PL+*#jOiru=8LWB3_5g};HR^-C{)9zAdqzHg&FxVYTk%_XMR#2{S6oX`2WOXU2RJZ!+5Ja+&2yl(J!e>tWKJLkpeJGrMKWo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
INSERT INTO public.users (id, name, email) OVERRIDING SYSTEM VALUE VALUES (10, 'user_10', '{"c": "mBbKxg#yys+)J>AoAAc@*xA*@FRc$hPyl}$7Hybcu36|L)=@aIA8qMf$zPtj;e*E>K+36)ln>kJ5U;ih0G^}-#2{?kRG82=yQl2LijGU+XrBMci7i!TKjn;^#B62|I4-9mWo=<;Y$Ct", "i": {"table": "users", "column": "email"}, "k": "ct", "m": null, "o": null, "u": null, "v": 1}');
