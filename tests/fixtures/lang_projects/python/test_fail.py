import unittest
from proof import add

class Failing(unittest.TestCase):
    def test_add(self):
        self.assertEqual(add(1, 1), 3)
