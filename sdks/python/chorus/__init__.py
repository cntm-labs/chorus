"""Chorus CPaaS SDK — SMS, Email, OTP with smart routing."""

from chorus.client import ChorusClient
from chorus.errors import ChorusError

__all__ = ["ChorusClient", "ChorusError"]
__version__ = "0.2.0"
