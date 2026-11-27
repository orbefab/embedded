#include <mbedtls/aes.h>
#include <mbedtls/gcm.h>
#include <mbedtls/pk.h>
#include <mbedtls/sha1.h>
#include <mbedtls/sha256.h>
#include <mbedtls/sha512.h>
#include <string.h>

static bool eq_hex(const uint8_t *got, size_t len, const char *want_hex) {
  static const char digits[] = "0123456789abcdef";
  if (strlen(want_hex) != len * 2) {
    return false;
  }
  for (size_t i = 0; i < len; i++) {
    if (digits[got[i] >> 4] != want_hex[2 * i] ||
        digits[got[i] & 0xf] != want_hex[2 * i + 1]) {
      return false;
    }
  }
  return true;
}

static bool hex_to_bytes(const char *hex, uint8_t *out, size_t out_len) {
  if (strlen(hex) != out_len * 2) {
    return false;
  }
  for (size_t i = 0; i < out_len; i++) {
    auto nibble = [](char c) -> int {
      if (c >= '0' && c <= '9') return c - '0';
      if (c >= 'a' && c <= 'f') return c - 'a' + 10;
      if (c >= 'A' && c <= 'F') return c - 'A' + 10;
      return -1;
    };
    int hi = nibble(hex[2 * i]);
    int lo = nibble(hex[2 * i + 1]);
    if (hi < 0 || lo < 0) return false;
    out[i] = (uint8_t)((hi << 4) | lo);
  }
  return true;
}

static bool check_sha() {
  uint8_t out[64];

  if (mbedtls_sha1((const unsigned char *)"abc", 3, out) != 0 ||
      !eq_hex(out, 20, "a9993e364706816aba3e25717850c26c9cd0d89d")) {
    Serial.println("sha1 fail");
    return false;
  }
  Serial.println("sha1 ok");

  if (mbedtls_sha256((const unsigned char *)"abc", 3, out, 0) != 0 ||
      !eq_hex(out, 32,
              "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")) {
    Serial.println("sha256 fail");
    return false;
  }
  Serial.println("sha256 ok");

  if (mbedtls_sha512((const unsigned char *)"abc", 3, out, 1) != 0 ||
      !eq_hex(out, 48,
              "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed"
              "8086072ba1e7cc2358baeca134c825a7")) {
    Serial.println("sha384 fail");
    return false;
  }
  Serial.println("sha384 ok");

  if (mbedtls_sha512((const unsigned char *)"abc", 3, out, 0) != 0 ||
      !eq_hex(out, 64,
              "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a"
              "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")) {
    Serial.println("sha512 fail");
    return false;
  }
  Serial.println("sha512 ok");

  // 512 bytes exceeds IDF's SHA DMA threshold (256 bytes) and exercises SHA-512 over GDMA.
  static uint8_t big_a[512];
  memset(big_a, 'A', sizeof(big_a));
  if (mbedtls_sha512(big_a, sizeof(big_a), out, 0) != 0 ||
      !eq_hex(out, 64,
              "b62bf048526a6e3bbe816dc5b3174f1160df6fa0422565c923a73a9dc1716ad8"
              "3bfde4dba9ba44ef60a741a943caeed7b4b91ce5b21b41115661deb253411bb1")) {
    Serial.println("sha512_dma fail");
    return false;
  }
  Serial.println("sha512_dma ok");

  return true;
}

static bool check_aes() {
  uint8_t key128[16], key256[32], iv0[16], iv[16];
  uint8_t pt[64], ct[64], dec[64];
  mbedtls_aes_context ctx;

  // FIPS 197 Appendix C.1 (AES-128) & C.3 (AES-256) ECB
  uint8_t ecb_pt[16], ecb_ct[16], ecb_dec[16];
  hex_to_bytes("00112233445566778899aabbccddeeff", ecb_pt, 16);

  hex_to_bytes("000102030405060708090a0b0c0d0e0f", key128, 16);
  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key128, 128);
  mbedtls_aes_crypt_ecb(&ctx, MBEDTLS_AES_ENCRYPT, ecb_pt, ecb_ct);
  mbedtls_aes_setkey_dec(&ctx, key128, 128);
  mbedtls_aes_crypt_ecb(&ctx, MBEDTLS_AES_DECRYPT, ecb_ct, ecb_dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ecb_ct, 16, "69c4e0d86a7b0430d8cdb78070b4c55a") ||
      memcmp(ecb_dec, ecb_pt, 16) != 0) {
    Serial.println("aes128_ecb fail");
    return false;
  }
  Serial.println("aes128_ecb ok");

  hex_to_bytes(
      "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
      key256, 32);
  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key256, 256);
  mbedtls_aes_crypt_ecb(&ctx, MBEDTLS_AES_ENCRYPT, ecb_pt, ecb_ct);
  mbedtls_aes_setkey_dec(&ctx, key256, 256);
  mbedtls_aes_crypt_ecb(&ctx, MBEDTLS_AES_DECRYPT, ecb_ct, ecb_dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ecb_ct, 16, "8ea2b7ca516745bfeafc49904b496089") ||
      memcmp(ecb_dec, ecb_pt, 16) != 0) {
    Serial.println("aes256_ecb fail");
    return false;
  }
  Serial.println("aes256_ecb ok");

  // NIST SP 800-38A keys, IV and 64-byte plaintext
  hex_to_bytes("2b7e151628aed2a6abf7158809cf4f3c", key128, 16);
  hex_to_bytes(
      "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
      key256, 32);
  hex_to_bytes("000102030405060708090a0b0c0d0e0f", iv0, 16);
  hex_to_bytes(
      "6bc1bee22e409f96e93d7e117393172a"
      "ae2d8a571e03ac9c9eb76fac45af8e51"
      "30c81c46a35ce411e5fbc1191a0a52ef"
      "f69f2445df4f9b17ad2b417be66c3710",
      pt, 64);

  // SP 800-38A F.2.1 CBC-AES128
  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key128, 128);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_ENCRYPT, 64, iv, pt, ct);
  mbedtls_aes_setkey_dec(&ctx, key128, 128);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_DECRYPT, 64, iv, ct, dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ct, 64,
              "7649abac8119b246cee98e9b12e9197d"
              "5086cb9b507219ee95db113a917678b2"
              "73bed6b8e3c1743b7116e69e22229516"
              "3ff1caa1681fac09120eca307586e1a7") ||
      memcmp(dec, pt, 64) != 0) {
    Serial.println("aes128_cbc fail");
    return false;
  }
  Serial.println("aes128_cbc ok");

  // SP 800-38A F.2.5 CBC-AES256
  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key256, 256);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_ENCRYPT, 64, iv, pt, ct);
  mbedtls_aes_setkey_dec(&ctx, key256, 256);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_DECRYPT, 64, iv, ct, dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ct, 64,
              "f58c4c04d6e5f1ba779eabfb5f7bfbd6"
              "9cfc4e967edb808d679f777bc6702c7d"
              "39f23369a9d9bacfa530e26304231461"
              "b2eb05e2c39be9fcda6c19078c6a9d1b") ||
      memcmp(dec, pt, 64) != 0) {
    Serial.println("aes256_cbc fail");
    return false;
  }
  Serial.println("aes256_cbc ok");

  // 2048-byte CBC exceeds IDF's AES interrupt threshold (2000 bytes), exercising AES IRQ + GDMA.
  static uint8_t big_pt[2048], big_ct[2048], big_dec[2048];
  for (size_t i = 0; i < sizeof(big_pt); i++) {
    big_pt[i] = (uint8_t)(i & 0xff);
  }
  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key256, 256);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_ENCRYPT, sizeof(big_pt), iv, big_pt, big_ct);
  mbedtls_aes_setkey_dec(&ctx, key256, 256);
  memcpy(iv, iv0, 16);
  mbedtls_aes_crypt_cbc(&ctx, MBEDTLS_AES_DECRYPT, sizeof(big_ct), iv, big_ct, big_dec);
  mbedtls_aes_free(&ctx);
  uint8_t ct_hash[32];
  mbedtls_sha256(big_ct, sizeof(big_ct), ct_hash, 0);
  if (!eq_hex(ct_hash, 32,
              "298c9556c94367cbeb2b21beda37f3b7526346078929e67ebdcde02955930bf2") ||
      memcmp(big_dec, big_pt, sizeof(big_pt)) != 0) {
    Serial.println("aes256_cbc_irq fail");
    return false;
  }
  Serial.println("aes256_cbc_irq ok");

  // SP 800-38A F.5.1 CTR-AES128 and F.5.5 CTR-AES256
  uint8_t ctr_iv0[16], nonce[16], stream[16];
  size_t nc_off;
  hex_to_bytes("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff", ctr_iv0, 16);

  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key128, 128);
  memcpy(nonce, ctr_iv0, 16);
  nc_off = 0;
  memset(stream, 0, 16);
  mbedtls_aes_crypt_ctr(&ctx, 64, &nc_off, nonce, stream, pt, ct);
  memcpy(nonce, ctr_iv0, 16);
  nc_off = 0;
  memset(stream, 0, 16);
  mbedtls_aes_crypt_ctr(&ctx, 64, &nc_off, nonce, stream, ct, dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ct, 64,
              "874d6191b620e3261bef6864990db6ce"
              "9806f66b7970fdff8617187bb9fffdff"
              "5ae4df3edbd5d35e5b4f09020db03eab"
              "1e031dda2fbe03d1792170a0f3009cee") ||
      memcmp(dec, pt, 64) != 0) {
    Serial.println("aes128_ctr fail");
    return false;
  }
  Serial.println("aes128_ctr ok");

  mbedtls_aes_init(&ctx);
  mbedtls_aes_setkey_enc(&ctx, key256, 256);
  memcpy(nonce, ctr_iv0, 16);
  nc_off = 0;
  memset(stream, 0, 16);
  mbedtls_aes_crypt_ctr(&ctx, 64, &nc_off, nonce, stream, pt, ct);
  memcpy(nonce, ctr_iv0, 16);
  nc_off = 0;
  memset(stream, 0, 16);
  mbedtls_aes_crypt_ctr(&ctx, 64, &nc_off, nonce, stream, ct, dec);
  mbedtls_aes_free(&ctx);
  if (!eq_hex(ct, 64,
              "601ec313775789a5b7a7f504bbf3d228"
              "f443e3ca4d62b59aca84e990cacaf5c5"
              "2b0930daa23de94ce87017ba2d84988d"
              "dfc9c58db67aada613c2dd08457941a6") ||
      memcmp(dec, pt, 64) != 0) {
    Serial.println("aes256_ctr fail");
    return false;
  }
  Serial.println("aes256_ctr ok");

  // NIST SP 800-38D AES-128-GCM Test Case 4 (uses HW AES-ECB + HW AES-CTR on ESP32-S3)
  uint8_t gcm_key[16], gcm_iv[12], gcm_aad[20], gcm_pt[60], gcm_ct[60], gcm_dec[60], tag[16];
  hex_to_bytes("feffe9928665731c6d6a8f9467308308", gcm_key, 16);
  hex_to_bytes("cafebabefacedbaddecaf888", gcm_iv, 12);
  hex_to_bytes("feedfacedeadbeeffeedfacedeadbeefabaddad2", gcm_aad, 20);
  hex_to_bytes(
      "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72"
      "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
      gcm_pt, 60);

  mbedtls_gcm_context gcm;
  mbedtls_gcm_init(&gcm);
  int rc = mbedtls_gcm_setkey(&gcm, MBEDTLS_CIPHER_ID_AES, gcm_key, 128);
  if (rc == 0) {
    rc = mbedtls_gcm_crypt_and_tag(
        &gcm, MBEDTLS_GCM_ENCRYPT, sizeof(gcm_pt), gcm_iv, sizeof(gcm_iv),
        gcm_aad, sizeof(gcm_aad), gcm_pt, gcm_ct, sizeof(tag), tag);
  }
  int dec_ok = -1, dec_bad = 0;
  bool dec_match = false;
  if (rc == 0) {
    dec_ok = mbedtls_gcm_auth_decrypt(
        &gcm, sizeof(gcm_ct), gcm_iv, sizeof(gcm_iv),
        gcm_aad, sizeof(gcm_aad), tag, sizeof(tag), gcm_ct, gcm_dec);
    dec_match = memcmp(gcm_dec, gcm_pt, sizeof(gcm_pt)) == 0;
    tag[0] ^= 1;
    uint8_t gcm_bad_dec[60];
    dec_bad = mbedtls_gcm_auth_decrypt(
        &gcm, sizeof(gcm_ct), gcm_iv, sizeof(gcm_iv),
        gcm_aad, sizeof(gcm_aad), tag, sizeof(tag), gcm_ct, gcm_bad_dec);
    tag[0] ^= 1;
  }
  mbedtls_gcm_free(&gcm);
  if (rc != 0 || dec_ok != 0 || !dec_match || dec_bad != MBEDTLS_ERR_GCM_AUTH_FAILED ||
      !eq_hex(gcm_ct, 60,
              "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e"
              "21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091") ||
      !eq_hex(tag, 16, "5bc94fbc3221a5db94fae95ae7121a47")) {
    Serial.println("aes128_gcm fail");
    return false;
  }
  Serial.println("aes128_gcm ok");

  return true;
}

static const char RSA2048_PUB_PEM[] =
    "-----BEGIN PUBLIC KEY-----\n"
    "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAv5abZekzQbe5uAcPvP9L\n"
    "33gN/T4RaevHQgQYTfg2yMbS7Wiy2sicjpiW7eHnLUlzPYq+Vkp9NqF1jQ3+0cj+\n"
    "dbiSG5C49LTJEa0WqnVM0m13Ru273nZ07Juwxr/L+DntvT03LNoaRzusDQrlecrc\n"
    "BCUmd2i3nPbnAbATkurEfSygnxhh6HrK6AiZNycQj+tyemdBz+XhXFrM077rCgq1\n"
    "pwLk2L6doeyO/PKD+Nn35Hw2YCHH9lQbzZtNpqOFeTmfrKb4FVn4Rf25y3fEE8LR\n"
    "ODWxWes4DNgoDBImPzGMAF0h5zypuan/ctq/HrXCQUuEpPfD3zekBfiIMwXSJeqT\n"
    "5QIDAQAB\n"
    "-----END PUBLIC KEY-----\n";

static const char RSA2048_SIG_HEX[] =
    "3faae2026281a967ce66e1818a14f16698ed070f49aa0b063808d03b1702cc9f"
    "94597da5a9dbc26fc166113ac8059ab20120162c55a27a27e3e683adad12a121"
    "1d954e4c534cf9822842ccab1a15a65bf5987f0eaeca173cdd6307e98d9ce681"
    "092edc2dd41802e79695f8a9c09fbb29a7f8e3209f94fb337fa739da972e0f59"
    "3dcf49e869ff7ece5ea1e90bd6ba89b7bd3ec9e5ad24d589674a37c3949be3f3"
    "4a47bf575a56182e39ca2ef58d815dc0e0bf9559a1b6aa1a8b9f3f404d463953"
    "c7d217128d96668ac26a04fc43943a0330aeb909975923efb0d9dff52f78557d"
    "137383984c460213acb424871dde078a6ab2641214b9c50fe9405f4186e029ff";

static bool check_rsa() {
  uint8_t hash[32];
  uint8_t sig[256];
  const char *msg = "orbe-esp32s3-rsa2048";

  if (mbedtls_sha256((const unsigned char *)msg, strlen(msg), hash, 0) != 0) {
    Serial.println("rsa2048_pk hash fail");
    return false;
  }
  hex_to_bytes(RSA2048_SIG_HEX, sig, sizeof(sig));

  mbedtls_pk_context pk;
  mbedtls_pk_init(&pk);
  int rc = mbedtls_pk_parse_public_key(
      &pk, (const unsigned char *)RSA2048_PUB_PEM, sizeof(RSA2048_PUB_PEM));
  if (rc != 0) {
    Serial.printf("rsa2048_pk parse fail %d\n", rc);
    mbedtls_pk_free(&pk);
    return false;
  }

  int ok = mbedtls_pk_verify(&pk, MBEDTLS_MD_SHA256, hash, sizeof(hash), sig, sizeof(sig));
  sig[sizeof(sig) - 1] ^= 1;
  int bad = mbedtls_pk_verify(&pk, MBEDTLS_MD_SHA256, hash, sizeof(hash), sig, sizeof(sig));
  mbedtls_pk_free(&pk);

  if (ok != 0 || bad == 0) {
    Serial.printf("rsa2048_pk fail ok=%d bad=%d\n", ok, bad);
    return false;
  }
  Serial.println("rsa2048_pk ok");
  return true;
}

void setup() {
  Serial.begin(115200);
  bool ok = check_sha() && check_aes() && check_rsa();
  Serial.println(ok ? "all crypto ok" : "crypto fail");
}

void loop() {
  delay(100);
}
