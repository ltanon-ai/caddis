from l1_a import GREETING
from l1_b import LIMIT
from l1_c import NAME
from l2_a import clamp
from l2_b import is_even
from l3_app import scaled

def test_l1():
    assert GREETING == "hello" and LIMIT == 10 and NAME == "caddis"

def test_l2():
    assert clamp(15, 0, 10) == 10 and is_even(4) and not is_even(3)

def test_l3():
    assert scaled(3) == 9
