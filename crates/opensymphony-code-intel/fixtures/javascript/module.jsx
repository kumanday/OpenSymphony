import React from "react";

export class Panel {
    render() {
        return React.createElement("section", null, "Ready");
    }
}

export function mount() {
    return <Panel />;
}

it("mounts", () => {
    mount();
});
