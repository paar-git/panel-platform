import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import LaunchScreen from './LaunchScreen';

describe('LaunchScreen', () => {
  it('shows the product while the core is starting', () => {
    render(<LaunchScreen />);
    expect(screen.getByRole('status')).toHaveTextContent('Panel Platform');
    expect(screen.getByRole('status')).toHaveTextContent('Starting');
  });

  it('replaces the progress with the failure when the core cannot start', () => {
    render(<LaunchScreen error="the database could not be opened" />);
    expect(screen.getByRole('status')).toHaveTextContent('the database could not be opened');
    expect(screen.getByRole('status')).not.toHaveTextContent('Starting');
  });
});
