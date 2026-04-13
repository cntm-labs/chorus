/** Error returned by the Chorus API. */
export class ChorusError extends Error {
  /** HTTP status code. */
  readonly status: number;
  /** Raw response body. */
  readonly body: string;

  constructor(status: number, body: string) {
    super(`Chorus API error (${status}): ${body}`);
    this.name = "ChorusError";
    this.status = status;
    this.body = body;
  }
}
