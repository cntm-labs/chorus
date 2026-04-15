package chorus

import "fmt"

// ChorusError is returned when the Chorus API returns an error response.
type ChorusError struct {
	// HTTP status code.
	Status int
	// Raw response body.
	Body string
}

func (e *ChorusError) Error() string {
	return fmt.Sprintf("Chorus API error (%d): %s", e.Status, e.Body)
}
