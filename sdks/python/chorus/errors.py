"""Error types for the Chorus SDK."""


class ChorusError(Exception):
    """Raised when the Chorus API returns an error response."""

    def __init__(self, status: int, body: str) -> None:
        super().__init__(f"Chorus API error ({status}): {body}")
        self.status = status
        self.body = body
