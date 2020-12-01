import hmac
import hashlib

def write_hmac(secret, output):
    h = hmac.new(bytearray(secret, encoding='utf8'), output.encode(), hashlib.sha256)
    with open(".tmc_test_results.hmac.sha256", "w") as text_file:
      text_file.write(h.hexdigest())
