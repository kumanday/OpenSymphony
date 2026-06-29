import React from "react";
import { render } from "@testing-library/react";

interface ButtonProps {
    label: string;
}

export function Button({ label }: ButtonProps) {
    return <button>{label.toUpperCase()}</button>;
}

const helper = () => render(<Button label="Save" />);

test("renders button", () => {
    helper();
});
